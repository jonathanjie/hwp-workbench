"""Preservation-oriented editing of existing HWP 5.x / HWPX text.

This alpha is deliberately NOT a complete word processor. It retains opaque
payloads, edits addressable text only, and rejects unsafe operations.
"""
from __future__ import annotations
from dataclasses import dataclass, asdict
from pathlib import PurePosixPath
from xml.parsers import expat
from xml.etree import ElementTree as ET
import hashlib, io, re, struct, zipfile, zlib
from .cfb import CompoundFile, MAGIC, u32
from .errors import WorkbenchError, require, MAX_FILE, MAX_PART, MAX_EXPANDED, MAX_NODES

HP = 'http://www.hancom.co.kr/hwpml/2011/paragraph'
HC = 'http://www.hancom.co.kr/hwpml/2011/core'
HH = 'http://www.hancom.co.kr/hwpml/2011/head'

# Observed in pyhwp-aligns.hwp and pyhwp-borderfill.hwp (public manifest).
# DefaultJScript contains length-prefixed UTF-16LE document bindings and an
# empty OnDocument_New callback with a //todo comment, plus native padding.
# JScriptVersion is the eight bytes 01 00 00 00 00 00 00 00.
DEFAULT_SCRIPT_HASHES = {
    'Scripts/DefaultJScript': 'e1f35ff38336598f79448c84b41bcb508d53a552808454a76ee12691cb2c97e4',
    'Scripts/JScriptVersion': '7c9fa136d4413fa6173637e883b6998d32e1d675f88cddff9dcbcf331820f4b8',
}

def sha(data: bytes) -> str: return hashlib.sha256(data).hexdigest()
def local(tag: str) -> str: return tag.rsplit('}',1)[-1]
def esc(s: str) -> str: return s.replace('&','&amp;').replace('<','&lt;').replace('>','&gt;')

def check_text(value):
    require(isinstance(value,str) and len(value)<=200_000,'BAD_PATCH','Text must be a string of at most 200,000 characters.')
    # Native tabs, paragraph breaks and fields need dedicated document operations.
    require(all((0x20 <= ord(c) <= 0xd7ff) or (0xe000 <= ord(c) <= 0xfffd) or (0x10000 <= ord(c) <= 0x10ffff) for c in value),
            'CONTROL_TEXT','Use plain text only; native tabs, newlines and control characters are not supported by this operation.')

@dataclass
class TextNode:
    id: str
    text: str
    part: str
    role: str
    editable: bool = True
    paragraph: int = 0
    style: dict | None = None

class Document:
    def __init__(self, data: bytes, name: str = 'document'):
        require(isinstance(data,bytes) and 0<len(data)<=MAX_FILE,'LIMIT','File must be between 1 byte and 64 MiB.')
        self.original = data
        self.name = name
        self.hash = sha(data)
        self.nodes: list[TextNode] = []
        self.warnings: list[dict] = []
        self.edits: dict[str,str] = {}
        self.positions = {}
        self.view = []
        self.block_save = False
        self.parts = {}
        if data[:8]==MAGIC:
            self.format='hwp'; self._read_hwp()
        elif data[:2]==b'PK':
            self.format='hwpx'; self._read_hwpx()
        else:
            raise WorkbenchError('UNSUPPORTED_FORMAT','Only HWP 5.x CFB and HWPX ZIP documents are supported. HWP 3.x is not supported.')
        require(len(self.nodes)<=MAX_NODES,'LIMIT','Too many text nodes.')
        self.by_id = {n.id:n for n in self.nodes}
        self.warn('LAYOUT_UNVERIFIED','warning','The workspace preview is approximate. Hancom layout, pagination and font fidelity have not been certified.')
        if self.block_save:
            for n in self.nodes: n.editable=False

    def warn(self, code, severity, message, **details):
        if not any(w['code']==code for w in self.warnings):
            self.warnings.append(dict(code=code,severity=severity,message=message,**details))

    def _xml(self, payload):
        require(len(payload)<=MAX_PART,'LIMIT','XML part exceeds 32 MiB.')
        try: text = payload.decode('utf-8-sig')
        except UnicodeError: raise WorkbenchError('XML_ENCODING','Only UTF-8 HWPX XML is supported in this alpha.')
        require(not re.search(r'<!\s*(DOCTYPE|ENTITY)\b',text,re.I),'UNSAFE_XML','DTD and entity declarations are prohibited.')
        require(not re.search(r'encoding\s*=\s*[\"\'](?!utf-8)[^\"\']+',text[:200],re.I),'XML_ENCODING','XML encoding must be UTF-8.')
        try: return ET.fromstring(payload)
        except ET.ParseError as e: raise WorkbenchError('BAD_XML',str(e))

    def _read_hwpx(self):
        try:
            with zipfile.ZipFile(io.BytesIO(self.original)) as z:
                self.zip_infos=z.infolist(); self.zip_comment=z.comment
                require(len(self.zip_infos)<=2048,'LIMIT','Too many ZIP members.')
                names=set(); total=0
                for info in self.zip_infos:
                    p=PurePosixPath(info.filename)
                    require(not p.is_absolute() and '..' not in p.parts and '\\' not in info.filename and '\x00' not in info.filename,
                            'UNSAFE_ZIP','Unsafe package path.')
                    require(info.filename not in names,'UNSAFE_ZIP','Duplicate ZIP member.')
                    require(not info.flag_bits&1,'ENCRYPTED','Encrypted ZIP members are not supported.')
                    require(info.compress_type in {zipfile.ZIP_STORED,zipfile.ZIP_DEFLATED},'UNSAFE_ZIP','Unsupported ZIP compression.')
                    names.add(info.filename); total+=info.file_size
                    require(info.file_size<=MAX_PART and total<=MAX_EXPANDED,'LIMIT','Expanded ZIP exceeds limits.')
                    self.parts[info.filename]=z.read(info)  # CRC checked by zipfile.
        except (zipfile.BadZipFile,RuntimeError,zlib.error) as e:
            raise WorkbenchError('BAD_ZIP',str(e))
        require(self.parts.get('mimetype',b'').strip()==b'application/hwp+zip','NOT_HWPX','Missing or invalid HWPX mimetype.')
        required={'Contents/header.xml','Contents/content.hpf'}
        require(required<=self.parts.keys(),'NOT_HWPX','Missing HWPX header or content manifest.')
        section_names=sorted((n for n in self.parts if re.fullmatch(r'Contents/section\d+\.xml',n)),key=lambda n:int(re.search(r'(\d+)\.xml$',n)[1]))
        require(bool(section_names),'NOT_HWPX','No body sections found.')
        trees={}
        for name,payload in self.parts.items():
            if name.endswith(('.xml','.hpf')):
                trees[name]=self._xml(payload)
            if any(s in name.lower() for s in ('signature','_xmlsignatures','scripts/','vbascript','drm/','encrypt')):
                self.block_save=True
                self.warn('PROTECTED_OR_ACTIVE_CONTENT','error','Signatures, encryption metadata or script content detected. Editing is blocked; these payloads are never executed.')
        head=trees['Contents/header.xml']
        styles={}
        for e in head.iter():
            if local(e.tag)=='charPr':
                h=e.attrib.get('height','1000')
                try: size=max(5,min(96,int(h)/100))
                except ValueError: size=10
                styles[e.attrib.get('id','0')]={'size':size,'bold':any(local(x.tag)=='bold' for x in e),'italic':any(local(x.tag)=='italic' for x in e)}
        self.fonts=[]
        for e in head.iter():
            if local(e.tag)=='font' and e.attrib.get('face'): self.fonts.append(e.attrib['face'])
        self.fonts=sorted(set(self.fonts))
        self.warn('FONT_METRICS_UNVERIFIED','warning','Requested fonts are listed, but installed font identity and text metrics are not verified. No fonts are bundled.')
        feature_names={'tbl':'TABLE_LAYOUT','pic':'IMAGES','equation':'EQUATIONS','ole':'OLE_OBJECTS','chart':'CHARTS','header':'HEADERS_FOOTERS','footer':'HEADERS_FOOTERS','footNote':'FOOTNOTES','endNote':'FOOTNOTES','fieldBegin':'FIELDS','rect':'SHAPES','line':'SHAPES','ellipse':'SHAPES','polygon':'SHAPES','curve':'SHAPES','container':'SHAPES','pageNum':'PAGE_NUMBERING','bookmark':'BOOKMARKS'}
        for section in section_names:
            root=trees[section]
            tokens=self._text_tokens(self.parts[section],section)
            token_by_element={}
            t_elements=[e for e in root.iter() if e.tag=='{'+HP+'}t']
            require(len(t_elements)==len(tokens),'BAD_XML','Text token mapping did not match namespace-aware parse.')
            for e,t in zip(t_elements,tokens): token_by_element[id(e)]=t
            para=0
            def scan(e,ancestors=(),style=None):
                nonlocal para
                require(len(ancestors)<128,'LIMIT','XML nesting exceeds 128 levels.')
                tag=local(e.tag)
                if tag in feature_names:
                    code=feature_names[tag]
                    self.warn(code,'warning',f'{tag} content is retained in the package; its full layout/behavior is not editable or certified in this alpha.')
                if tag=='p': para+=1
                if tag=='run': style=styles.get(e.attrib.get('charPrIDRef','0'),{})
                if id(e) in token_by_element:
                    t=token_by_element[id(e)]
                    role='table-cell' if 'tc' in ancestors else ('header-footer' if any(k in ancestors for k in ('header','footer')) else 'body')
                    allowed=t['plain'] and not any(k in ancestors for k in ('equation','chart','ole','fieldBegin','fieldEnd'))
                    node=TextNode(t['id'],''.join(e.itertext()),section,role,allowed,para,style or {})
                    self.nodes.append(node); self.positions[node.id]=t
                for c in e: scan(c,ancestors+(tag,),style)
            scan(root)
            def visual(e, depth=0):
                if depth > 32:
                    return []
                tag = local(e.tag)
                if tag == 't' and id(e) in token_by_element:
                    return [{'type': 'text', 'node': token_by_element[id(e)]['id']}]
                if tag in {'pic', 'ole', 'chart', 'equation', 'rect', 'line', 'ellipse', 'polygon', 'curve'}:
                    return [{'type': 'opaque', 'label': tag}]
                children = []
                for child in e:
                    children.extend(visual(child, depth + 1))
                if tag == 'tbl':
                    return [{'type': 'table', 'children': children}]
                if tag == 'tr':
                    return [{'type': 'row', 'children': children}]
                if tag == 'tc':
                    span = next((child for child in e if local(child.tag) == 'cellSpan'), None)
                    try:
                        columns = int(span.attrib.get('colSpan', '1')) if span is not None else 1
                        rows = int(span.attrib.get('rowSpan', '1')) if span is not None else 1
                    except ValueError:
                        columns = 1
                        rows = 1
                    native_span = None if span is None else {
                        'rows': span.attrib.get('rowSpan', '1'),
                        'columns': span.attrib.get('colSpan', '1'),
                    }
                    address = next((child for child in e if local(child.tag) == 'cellAddr'), None)
                    # Retain raw addresses so agent operations can refuse malformed positions.
                    position = None if address is None else {
                        'row': address.attrib.get('rowAddr'),
                        'column': address.attrib.get('colAddr'),
                    }
                    return [{'type': 'cell', 'colspan': max(1, min(100, columns)),
                             'rowspan': max(1, min(100, rows)), 'address': position,
                             'nativeSpan': native_span, 'children': children}]
                if tag == 'p':
                    return [{'type': 'paragraph', 'children': children}]
                return children
            self.view.append({'type':'section','label':section,'children':visual(root)})
        self.warn('PREVIEW_CACHE_RETAINED','info','Embedded preview images/text and line-layout caches are retained unchanged. They may be stale after edits; reopen in Hancom to refresh.')

    def _text_tokens(self,data,part):
        parser=expat.ParserCreate(namespace_separator='}')
        tokens=[]; active=[]; depth=0; count=0
        def tag_end(start):
            quote=None
            for i in range(start,len(data)):
                b=data[i]
                if quote:
                    if b==quote: quote=None
                elif b in (34,39): quote=b
                elif b==62: return i+1
            raise WorkbenchError('BAD_XML','Unterminated XML tag.')
        def start(name,attrs):
            nonlocal depth,count
            depth+=1;count+=1
            require(depth<=128 and count<=MAX_NODES*10,'LIMIT','XML complexity limit reached.')
            for a in active: a['plain']=False
            if name==HP+'}t':
                at=parser.CurrentByteIndex; end=tag_end(at); raw=data[at:end]
                qname=raw[1:].split(None,1)[0].split(b'>',1)[0].rstrip(b'/').decode('ascii')
                t={'id':f'{part}#text:{len(tokens)}','start':at,'inner_start':end,'plain':True,'selfclose':raw.rstrip().endswith(b'/>'),'qname':qname}
                tokens.append(t);active.append(t)
        def end(name):
            nonlocal depth
            if name==HP+'}t':
                t=active.pop();at=parser.CurrentByteIndex
                t['inner_end']=t['inner_start'] if t['selfclose'] else at
                t['end']=t['inner_start'] if t['selfclose'] else tag_end(at)
            depth-=1
        parser.StartElementHandler=start;parser.EndElementHandler=end
        parser.StartDoctypeDeclHandler=lambda *a: (_ for _ in ()).throw(WorkbenchError('UNSAFE_XML','DTD prohibited.'))
        try: parser.Parse(data,True)
        except expat.ExpatError as e: raise WorkbenchError('BAD_XML',str(e))
        return tokens

    def _hwp_stream(self, name):
        payload = self.parts[name]
        self.stream_trailers[name] = False
        if not self.compressed:
            require(len(payload) <= MAX_PART, 'LIMIT', 'HWP stream exceeds limit.')
            return payload
        try:
            decompressor = zlib.decompressobj(-15)
            raw = decompressor.decompress(payload, MAX_PART + 1)
        except zlib.error as error:
            raise WorkbenchError('BAD_HWP_COMPRESSION', str(error))
        message = 'Invalid or oversized compressed HWP stream.'
        require(
            len(raw) <= MAX_PART and not decompressor.unconsumed_tail and decompressor.eof,
            'LIMIT', message,
        )
        trailer = decompressor.unused_data
        if trailer:
            # Some Hancom raw-deflate streams end with CRC32 and expanded size,
            # both unsigned 32-bit little-endian. No other suffix is accepted.
            require(len(trailer) == 8, 'LIMIT', message)
            checksum, size = struct.unpack('<II', trailer)
            require(checksum == zlib.crc32(raw) and size == len(raw), 'LIMIT', message)
            self.stream_trailers[name] = True
        return raw

    def _read_hwp(self):
        self.cfb = CompoundFile(self.original)
        self.parts = self.cfb.streams
        head = self.parts.get('FileHeader', b'')
        require(len(head) >= 40 and head[:17] == b'HWP Document File', 'NOT_HWP', 'Missing HWP FileHeader.')
        require(head[35] == 5, 'UNSUPPORTED_FORMAT', 'Only HWP 5.x is supported.')
        self.flags = u32(head, 36)
        protected_flags = (1 << 1) | (1 << 2) | (1 << 4) | (1 << 8) | (1 << 9)
        require(not self.flags & protected_flags, 'PROTECTED_HWP', 'Password, distribution or DRM protected HWP cannot be opened by this alpha.')
        self.compressed = bool(self.flags & 1)
        self.stream_trailers = {}
        self._classify_hwp_scripts()
        self.sections = {}
        self.fonts = []
        expanded_total = 0
        section_names = sorted(
            (name for name in self.parts if re.fullmatch(r'BodyText/Section\d+', name)),
            key=lambda name: int(name.rsplit('Section', 1)[1]),
        )
        for name in section_names:
            raw = self._hwp_stream(name)
            expanded_total += len(raw)
            require(expanded_total <= MAX_EXPANDED, 'LIMIT', 'Expanded HWP sections exceed limit.')
            self.sections[name] = raw
            self._read_hwp_section(name, raw)
        require(bool(self.sections), 'NOT_HWP', 'No HWP body sections found.')
        self.warn('HWP_EQUAL_LENGTH_ONLY', 'warning', 'HWP alpha editing requires the same UTF-16 code-unit length per text run. Length-changing and structural edits are blocked to retain character/line offsets.')
        self.warn('HWP_LAYOUT_NOT_RENDERED', 'warning', 'HWP is shown as structured text, not a page-faithful rendering. Table/control/style streams are retained but not visually reconstructed.')
        self.warn('PREVIEW_CACHE_RETAINED', 'info', 'Stored preview content remains unchanged and may be stale after editing.')

    def _classify_hwp_scripts(self):
        script_names = {name for name in self.parts if 'script' in name.lower()}
        known_stub = False
        if script_names == set(DEFAULT_SCRIPT_HASHES):
            # Hash native bytes only. Never decode, interpret or execute scripts.
            payloads = {name: self._hwp_stream(name) for name in sorted(script_names)}
            known_stub = all(sha(payloads[name]) == digest for name, digest in DEFAULT_SCRIPT_HASHES.items())
        if known_stub:
            self.warn('DEFAULT_SCRIPT_STUB', 'info', 'Exact known Hancom default script stub retained byte-identically; never executed.')
        has_scripts = bool(script_names) or bool(self.flags & (1 << 3))
        has_signature = bool(self.flags & ((1 << 6) | (1 << 7)))
        if has_signature or (has_scripts and not known_stub):
            self.block_save = True
            self.warn('PROTECTED_OR_ACTIVE_CONTENT', 'error', 'Script or digital-signature metadata detected; editing is blocked.')

    def _read_hwp_section(self, name, raw):
        at = 0
        record = 0
        paragraph = 0
        children = []
        while at < len(raw):
            require(at + 4 <= len(raw), 'BAD_HWP_RECORD', 'Truncated record header.')
            bits = u32(raw, at)
            tag = bits & 1023
            size = bits >> 20
            header_size = 4
            if size == 4095:
                require(at + 8 <= len(raw), 'BAD_HWP_RECORD', 'Truncated extended record header.')
                size = u32(raw, at + 4)
                header_size = 8
            start = at + header_size
            end = start + size
            require(end <= len(raw), 'BAD_HWP_RECORD', 'Record payload exceeds stream bounds.')
            if tag == 66:
                paragraph += 1
            if tag == 67:
                require(size % 2 == 0, 'BAD_HWP_TEXT', 'Misaligned UTF-16 text record.')
                # Edit only contiguous printable text; native controls stay opaque.
                offset = start
                segment = 0
                while offset < end:
                    code = struct.unpack_from('<H', raw, offset)[0]
                    if code < 32:
                        width = 16 if (1 <= code <= 9 or 11 <= code <= 12 or 14 <= code <= 23) else 2
                        require(offset + width <= end, 'BAD_HWP_TEXT', 'Truncated HWP control sequence.')
                        offset += width
                        continue
                    begin = offset
                    while offset < end and struct.unpack_from('<H', raw, offset)[0] >= 32:
                        offset += 2
                    try:
                        text = raw[begin:offset].decode('utf-16le')
                    except UnicodeError:
                        raise WorkbenchError('BAD_HWP_TEXT', 'Invalid UTF-16 in paragraph text.')
                    node_id = f'{name}#record:{record}:text:{segment}'
                    self.nodes.append(TextNode(node_id, text, name, 'text-run', True, paragraph, {}))
                    self.positions[node_id] = {'start': begin, 'end': offset, 'part': name}
                    children.append({'type': 'paragraph', 'children': [{'type': 'text', 'node': node_id}]})
                    segment += 1
            at = end
            record += 1
            require(record <= MAX_NODES, 'LIMIT', 'Too many HWP records.')
        self.view.append({'type': 'section', 'label': name, 'children': children})

    def inspect(self):
        return dict(schemaVersion=1,format=self.format,name=self.name,sha256=self.hash,
                    nodes=[asdict(n)|{'text':self.edits.get(n.id,n.text)} for n in self.nodes],
                    warnings=self.warnings,view=self.view,fonts=self.fonts,
                    capabilities={'textEdit':not self.block_save,'lengthChangingText':self.format=='hwpx' and not self.block_save,
                                  'structuralEdit':False,'formattingEdit':False,'certifiedLayout':False,'saveAs':not self.block_save},
                    parts=[{'name':k,'size':len(v),'sha256':sha(v)} for k,v in self.parts.items()],
                    pendingChanges=len(self.edits))

    def patch(self,patch: dict):
        require(isinstance(patch,dict) and patch.get('schemaVersion')==1,'BAD_PATCH','Expected patch schemaVersion 1.')
        require(patch.get('baseSha256')==self.hash,'STALE_DOCUMENT','Patch hash differs from source document; re-inspect before editing.')
        operations=patch.get('operations')
        require(isinstance(operations,list) and 0<len(operations)<=1000,'BAD_PATCH','Supply 1–1000 operations.')
        require(not self.block_save,'SAVE_BLOCKED','Protected/active content blocks edits.')
        pending=dict(self.edits);seen=set()
        for op in operations:
            require(isinstance(op,dict) and set(op)=={'op','nodeId','expectedText','text'},'BAD_PATCH','Each operation requires exactly op, nodeId, expectedText, text.')
            require(op['op']=='set_text' and isinstance(op['nodeId'],str),'BAD_PATCH','Only set_text is implemented.')
            nid=op['nodeId'];require(nid not in seen,'BAD_PATCH','Duplicate target in patch.');seen.add(nid)
            require(nid in self.by_id,'NODE_NOT_FOUND','Unknown node ID: '+nid)
            node=self.by_id[nid]
            require(node.editable,'UNSUPPORTED_ELEMENT','This text is not safely editable.')
            require(op['expectedText']==pending.get(nid,node.text),'STALE_TEXT','Expected text no longer matches '+nid)
            value=op['text'];check_text(value)
            if self.format=='hwp':
                require(len(value.encode('utf-16le'))==len(node.text.encode('utf-16le')),'HWP_LENGTH_CHANGE_BLOCKED','HWP alpha requires equal UTF-16 length. No output was written.')
            if value==node.text: pending.pop(nid,None)
            else: pending[nid]=value
        # All-or-nothing; no mutations until every operation has passed.
        self.edits=pending
        return self.diff()

    def diff(self):
        return {'baseSha256':self.hash,'changes':[{'nodeId':k,'before':self.by_id[k].text,'after':v,'part':self.by_id[k].part} for k,v in self.edits.items()],
                'warnings':self.warnings,'layoutCertified':False}

    def serialize(self, acknowledge_risks=False):
        if not self.edits:
            return self.original
        require(not self.block_save, 'SAVE_BLOCKED', 'Saving is blocked.')
        require(acknowledge_risks, 'RISK_ACK_REQUIRED', 'Edited output requires explicit acknowledgement of compatibility and stale-preview risks.')
        changed = {}
        by_part = {}
        for node_id, value in self.edits.items():
            by_part.setdefault(self.by_id[node_id].part, []).append((node_id, value))
        for part, items in by_part.items():
            raw = self.parts[part] if self.format == 'hwpx' else self.sections[part]
            replacements = []
            for node_id, value in items:
                position = self.positions[node_id]
                if self.format == 'hwp':
                    replacements.append((position['start'], position['end'], value.encode('utf-16le')))
                elif position['selfclose']:
                    opening = raw[position['start']:position['inner_start']]
                    new = opening[:-2] + b'>' + esc(value).encode() + b'</' + position['qname'].encode() + b'>'
                    replacements.append((position['start'], position['end'], new))
                else:
                    replacements.append((position['inner_start'], position['inner_end'], esc(value).encode('utf-8')))
            for start, end, new in sorted(replacements, reverse=True):
                raw = raw[:start] + new + raw[end:]
            if self.format == 'hwpx':
                self._xml(raw)
            elif self.compressed:
                compressor = zlib.compressobj(level=6, wbits=-15)
                compressed = compressor.compress(raw) + compressor.flush()
                if self.stream_trailers[part]:
                    compressed += struct.pack('<II', zlib.crc32(raw), len(raw))
                raw = compressed
            changed[part] = raw
        if self.format == 'hwp':
            result = self.cfb.rewrite(changed)
        else:
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, 'w') as archive:
                archive.comment = self.zip_comment
                for info in self.zip_infos:
                    archive.writestr(info, changed.get(info.filename, self.parts[info.filename]))
            result = buffer.getvalue()
        check = Document(result, self.name)
        require(set(check.parts) == set(self.parts), 'VERIFY_FAILED', 'Package inventory changed.')
        for name, payload in self.parts.items():
            if name not in changed:
                require(check.parts[name] == payload, 'VERIFY_FAILED', 'Opaque part changed: ' + name)
        for node_id, text in self.edits.items():
            require(node_id in check.by_id and check.by_id[node_id].text == text, 'VERIFY_FAILED', 'Edit was not preserved on reopen: ' + node_id)
        return result
