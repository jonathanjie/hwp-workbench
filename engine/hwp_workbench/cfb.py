"""Bounded CFB reader and same-directory stream rewriter.

No HWP controls are synthesized here. Directory topology, metadata and opaque
stream payloads are retained. A no-op returns the original file verbatim.
Only CFB v3/v4, 512/4096-byte sectors and 64-byte mini sectors are accepted.
"""
from __future__ import annotations
import struct
from .errors import require, MAX_FILE, MAX_EXPANDED

MAGIC = bytes.fromhex('d0cf11e0a1b11ae1')
FREE = 0xffffffff
END = 0xfffffffe
FAT = 0xfffffffd
DIF = 0xfffffffc

def u16(b, at): return struct.unpack_from('<H', b, at)[0]
def u32(b, at): return struct.unpack_from('<I', b, at)[0]
def u64(b, at): return struct.unpack_from('<Q', b, at)[0]
def p32(b, at, v): struct.pack_into('<I', b, at, v)
def p64(b, at, v): struct.pack_into('<Q', b, at, v)
def words(b):
    require(len(b) % 4 == 0, 'BAD_CFB', 'Misaligned allocation table.')
    return list(struct.unpack('<' + 'I' * (len(b) // 4), b))

class CompoundFile:
    def __init__(self, data: bytes):
        require(512 <= len(data) <= MAX_FILE and data[:8] == MAGIC, 'BAD_CFB', 'Invalid CFB signature or size.')
        require(u16(data, 28) == 0xfffe, 'BAD_CFB', 'Unsupported byte order.')
        self.major = u16(data, 26)
        shift = u16(data, 30)
        require((self.major, shift) in {(3, 9), (4, 12)}, 'BAD_CFB', 'Unsupported CFB version/sector size.')
        require(u16(data, 32) == 6 and u32(data, 56) == 4096, 'BAD_CFB', 'Unsupported mini stream parameters.')
        self.data = data
        self.ss = 1 << shift
        require(len(data) % self.ss == 0, 'BAD_CFB', 'Truncated CFB sector.')
        self.nsectors = len(data) // self.ss - 1
        require(self.nsectors > 0, 'BAD_CFB', 'CFB has no sectors.')
        fat_ids = [x for x in words(data[76:512]) if x != FREE]
        did = u32(data, 68)
        seen = set()
        ndif = u32(data, 72)
        require(ndif <= self.nsectors, 'BAD_CFB', 'DIFAT count exceeds file bounds.')
        for _ in range(ndif):
            require(did not in seen, 'BAD_CFB', 'Cyclic DIFAT.')
            seen.add(did)
            w = words(self.sector(did))
            fat_ids.extend(x for x in w[:-1] if x != FREE)
            did = w[-1]
        nfat = u32(data, 44)
        require(nfat <= self.nsectors and len(fat_ids) == nfat and len(set(fat_ids)) == nfat,
                'BAD_CFB', 'Invalid FAT count or duplicate FAT sector.')
        self.fat = []
        for sid in fat_ids:
            self.fat.extend(words(self.sector(sid)))
        self.directory = self.read_chain(u32(data, 48), self.fat)
        require(len(self.directory) % 128 == 0, 'BAD_CFB', 'Truncated directory.')
        require(len(self.directory)//128 <= 8192, 'LIMIT', 'Too many CFB directory entries.')
        self.entries = []
        for i in range(0, len(self.directory), 128):
            raw = self.directory[i:i+128]
            kind = raw[66]
            n = u16(raw, 64)
            require(kind in {0, 1, 2, 5}, 'BAD_CFB', 'Unknown directory entry kind.')
            require(kind == 0 or (2 <= n <= 64 and n % 2 == 0), 'BAD_CFB', 'Invalid directory name length.')
            try:
                name = raw[:max(0, n-2)].decode('utf-16le') if kind else ''
            except UnicodeError:
                require(False, 'BAD_CFB', 'Invalid directory name encoding.')
            self.entries.append(dict(raw=raw, name=name, kind=kind, left=u32(raw,68), right=u32(raw,72), child=u32(raw,76), start=u32(raw,116), size=u64(raw,120)))
        require(bool(self.entries) and self.entries[0]['kind'] == 5, 'BAD_CFB', 'Missing CFB root.')
        root = self.entries[0]
        require(root['size'] <= MAX_EXPANDED, 'BAD_CFB', 'Mini stream too large.')
        self.mini = self.read_chain(root['start'], self.fat, root['size'])
        nmini = u32(data,64)
        require(nmini <= self.nsectors, 'BAD_CFB', 'MiniFAT too large.')
        self.minifat = words(self.read_chain(u32(data,60), self.fat, nmini*self.ss)) if nmini else []
        self.paths = {}
        walked = set()
        path_keys = set()
        todo = [(root['child'], '')]
        while todo:
            idx, prefix = todo.pop()
            if idx == FREE: continue
            require(idx < len(self.entries) and idx not in walked, 'BAD_CFB', 'Invalid or cyclic directory tree.')
            walked.add(idx)
            e = self.entries[idx]
            require(e['kind'] in {1, 2}, 'BAD_CFB', 'Invalid directory member.')
            require('/' not in e['name'] and e['name'] not in {'.','..',''}, 'BAD_CFB', 'Unsafe directory name.')
            path = prefix + e['name']
            require(path.casefold() not in path_keys, 'BAD_CFB', 'Duplicate stream name.')
            path_keys.add(path.casefold())
            self.paths[path] = idx
            todo.extend([(e['right'],prefix),(e['left'],prefix)])
            if e['kind'] == 1: todo.append((e['child'], path + '/'))
        self.streams = {}
        total = 0
        for path, idx in self.paths.items():
            e = self.entries[idx]
            if e['kind'] != 2: continue
            total += e['size']
            require(total <= MAX_EXPANDED, 'BAD_CFB', 'Expanded streams exceed limit.')
            self.streams[path] = self.read_chain(e['start'], self.minifat, e['size'], mini=True) if e['size'] < 4096 else self.read_chain(e['start'], self.fat, e['size'])

    def sector(self, sid):
        require(0 <= sid < self.nsectors, 'BAD_CFB', 'Sector pointer is out of bounds.')
        return self.data[(sid+1)*self.ss:(sid+2)*self.ss]

    def read_chain(self, start, table, size=None, mini=False):
        if size == 0: return b''
        chain, seen = [], set()
        sid = start
        unit = 64 if mini else self.ss
        limit = len(self.mini)//64 if mini else self.nsectors
        while sid != END:
            require(sid not in seen and 0 <= sid < len(table) and sid < limit,
                    'BAD_CFB', 'Invalid or cyclic sector chain.')
            seen.add(sid)
            chain.append(self.mini[sid*64:(sid+1)*64] if mini else self.sector(sid))
            require(len(chain)*unit <= MAX_EXPANDED, 'BAD_CFB', 'Sector chain exceeds limit.')
            sid = table[sid]
        result = b''.join(chain)
        if size is not None:
            require(size <= len(result) and len(result) < size+unit, 'BAD_CFB', 'Stream size does not match its chain.')
            return result[:size]
        return result

    def rewrite(self, changed: dict[str, bytes]) -> bytes:
        require(set(changed) <= set(self.streams), 'BAD_CFB', 'Rewriter cannot add streams.')
        changed = {k:v for k,v in changed.items() if v != self.streams[k]}
        if not changed: return self.data
        # Keep original directory indices, links, class IDs, flags, and timestamps.
        directory = bytearray(self.directory)
        ss = self.ss
        sectors, allocation = [], []
        def allocate(payload):
            if not payload: return END
            first = len(sectors)
            count = (len(payload)+ss-1)//ss
            for n in range(count):
                sectors.append(payload[n*ss:(n+1)*ss].ljust(ss,b'\0'))
                allocation.append(first+n+1 if n+1<count else END)
            return first
        minidata = bytearray()
        minialloc = []
        for path, idx in sorted(self.paths.items(), key=lambda item:item[1]):
            if path not in self.streams: continue
            payload = changed.get(path, self.streams[path])
            require(len(payload) <= MAX_EXPANDED, 'LIMIT', 'Stream exceeds limit.')
            if 0 < len(payload) < 4096:
                start = len(minialloc)
                count = (len(payload)+63)//64
                minidata.extend(payload.ljust(count*64,b'\0'))
                minialloc.extend(start+i+1 if i+1<count else END for i in range(count))
            else:
                start = allocate(payload)
            p32(directory,idx*128+116,start)
            p64(directory,idx*128+120,len(payload))
        p32(directory,116,allocate(bytes(minidata)))
        p64(directory,120,len(minidata))
        if minialloc:
            mini_bytes = b''.join(struct.pack('<I',v) for v in minialloc)
            mini_bytes += b'\xff' * ((-len(mini_bytes)) % ss)
            mini_start = allocate(mini_bytes)
            mini_count = len(mini_bytes)//ss
        else:
            mini_start,mini_count = END,0
        dir_start = allocate(bytes(directory))
        # Number of FAT sectors includes FAT/DIFAT themselves.
        entries_per_fat = ss//4
        nfat, ndif = 0,0
        while True:
            nextfat = (len(sectors)+nfat+ndif+entries_per_fat-1)//entries_per_fat
            nextdif = max(0, (nextfat-109+(entries_per_fat-2))//(entries_per_fat-1))
            if (nextfat,nextdif)==(nfat,ndif): break
            nfat,ndif = nextfat,nextdif
        fat_ids = list(range(len(sectors),len(sectors)+nfat))
        dif_ids = list(range(len(sectors)+nfat,len(sectors)+nfat+ndif))
        table = allocation + [FAT]*nfat + [DIF]*ndif
        table += [FREE]*(nfat*entries_per_fat-len(table))
        for i in range(nfat):
            sectors.append(struct.pack('<'+'I'*entries_per_fat,*table[i*entries_per_fat:(i+1)*entries_per_fat]))
        extra = fat_ids[109:]
        for i in range(ndif):
            block = extra[i*(entries_per_fat-1):(i+1)*(entries_per_fat-1)]
            block += [FREE]*(entries_per_fat-1-len(block))
            block.append(dif_ids[i+1] if i+1<ndif else END)
            sectors.append(struct.pack('<'+'I'*entries_per_fat,*block))
        header = bytearray(self.data[:ss])
        p32(header,40,len(directory)//ss if self.major==4 else 0)
        p32(header,44,nfat); p32(header,48,dir_start)
        p32(header,60,mini_start); p32(header,64,mini_count)
        p32(header,68,dif_ids[0] if dif_ids else END); p32(header,72,ndif)
        header[76:512] = struct.pack('<'+'I'*109,*(fat_ids[:109]+[FREE]*max(0,109-len(fat_ids))))
        result = bytes(header)+b''.join(sectors)
        require(len(result)<=MAX_FILE,'LIMIT','Output file exceeds limit.')
        # Reparse before publishing and verify EVERY opaque stream.
        check = CompoundFile(result)
        require(set(check.streams)==set(self.streams),'VERIFY_FAILED','CFB stream inventory changed.')
        for name, old in self.streams.items():
            require(check.streams[name] == changed.get(name,old), 'VERIFY_FAILED', 'CFB stream did not survive serialization: '+name)
        return result
