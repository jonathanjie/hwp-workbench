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
from .hwp_records import parse_section, rewrite_section, validate_text_edit
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
    anchor: dict | None = None
    paragraphRecordIndex: int | None = None
    textStart: int | None = None
    textEnd: int | None = None

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
        self.references = self._references()
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
        for section_index, section in enumerate(section_names):
            root=trees[section]
            tokens, empty_control_indexes = self._text_tokens(self.parts[section], section)
            empty_controls = {id(element) for index, element in enumerate(root.iter())
                              if index in empty_control_indexes}
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
                    node=TextNode(t['id'],self._hwpx_text(e, empty_controls),section,role,allowed,para,style or {})
                    self.nodes.append(node); self.positions[node.id]=t
                for c in e: scan(c,ancestors+(tag,),style)
            scan(root)
            self._hwpx_anchors(root, token_by_element, section_index, empty_controls)
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

    @staticmethod
    def _hwpx_text(element, empty_controls):
        # rhwp's text API counts Unicode scalars, including these inline text
        # controls. Object and field slots do not occupy its visible text axis.
        inline_text = {
            'lineBreak': '\n', 'columnBreak': '\n', 'tab': '\t',
            'nbSpace': '\u00a0', 'fwSpace': '\u2007', 'hyphen': '\u00ad',
        }
        text = element.text or ''
        for child in element:
            # rhwp 0.8.7 only emits inline controls from self-closing XML
            # events. ElementTree alone cannot retain that lexical distinction.
            tag = local(child.tag)
            if tag in inline_text and id(child) in empty_controls:
                text += inline_text[tag]
            else:
                text += Document._hwpx_text(child, empty_controls)
            text += child.tail or ''
        return text

    def _hwpx_anchors(self, root, tokens, section_index, empty_controls):
        token_ids = {token['id'] for token in tokens.values()}
        nodes = {node.id: node for node in self.nodes if node.id in token_ids}
        for node in nodes.values():
            node.anchor = {'section': section_index, 'paragraph': 0,
                           'charOffset': None, 'inCell': node.role == 'table-cell'}
        paragraphs = [child for child in root if child.tag == '{' + HP + '}p']
        if root.tag == '{' + HP + '}p':
            paragraphs = [root]
        for paragraph_index, paragraph in enumerate(paragraphs):
            char_offset = 0

            def visit(element, top_level=True, in_cell=False, parent_tag=None):
                nonlocal char_offset
                if element is not paragraph and element.tag == '{' + HP + '}p':
                    top_level = False
                in_cell = in_cell or element.tag == '{' + HP + '}tc'
                token = tokens.get(id(element))
                if token is not None:
                    node = nodes[token['id']]
                    node.anchor = {'section': section_index, 'paragraph': paragraph_index,
                                   'charOffset': char_offset if top_level else None, 'inCell': in_cell}
                    if top_level:
                        char_offset += len(node.text)
                    elif node.role == 'body':
                        node.role = 'text-box'
                    return
                tag = local(element.tag)
                if top_level:
                    # Auto-number controls emit one space from either XML form
                    # inside ctrl; standalone tabs and breaks require Empty events.
                    if tag == 'autoNum' and parent_tag == 'ctrl':
                        char_offset += 1
                    elif id(element) in empty_controls and tag in {'lineBreak', 'columnBreak', 'softHyphen', 'tab'}:
                        char_offset += 1
                for child in element:
                    visit(child, top_level, in_cell, tag)

            visit(paragraph)

    def _text_tokens(self, data, part):
        parser = expat.ParserCreate(namespace_separator='}')
        tokens = []
        active = []
        depth = 0
        count = 0
        empty_control_indexes = set()
        control_names = {HP + '}' + tag for tag in
                         ('lineBreak', 'columnBreak', 'softHyphen', 'tab', 'autoNum', 'nbSpace', 'fwSpace', 'hyphen')}

        def tag_end(start):
            quote = None
            for index in range(start, len(data)):
                byte = data[index]
                if quote:
                    if byte == quote:
                        quote = None
                elif byte in (34, 39):
                    quote = byte
                elif byte == 62:
                    return index + 1
            raise WorkbenchError('BAD_XML', 'Unterminated XML tag.')

        def start(name, attrs):
            nonlocal depth, count
            depth += 1
            count += 1
            require(depth <= 128 and count <= MAX_NODES * 10, 'LIMIT', 'XML complexity limit reached.')
            if name in control_names:
                at = parser.CurrentByteIndex
                opening = data[at:tag_end(at)]
                if opening.rstrip().endswith(b'/>'):
                    empty_control_indexes.add(count - 1)
            for token in active:
                token['plain'] = False
            if name == HP + '}t':
                at = parser.CurrentByteIndex
                end = tag_end(at)
                opening = data[at:end]
                qname = opening[1:].split(None, 1)[0].split(b'>', 1)[0].rstrip(b'/').decode('ascii')
                token = {
                    'id': f'{part}#text:{len(tokens)}', 'start': at,
                    'inner_start': end, 'plain': True,
                    'selfclose': opening.rstrip().endswith(b'/>'), 'qname': qname,
                }
                tokens.append(token)
                active.append(token)

        def end(name):
            nonlocal depth
            if name == HP + '}t':
                token = active.pop()
                at = parser.CurrentByteIndex
                token['inner_end'] = token['inner_start'] if token['selfclose'] else at
                token['end'] = token['inner_start'] if token['selfclose'] else tag_end(at)
            depth -= 1

        def reject_doctype(*args):
            raise WorkbenchError('UNSAFE_XML', 'DTD prohibited.')

        parser.StartElementHandler = start
        parser.EndElementHandler = end
        parser.StartDoctypeDeclHandler = reject_doctype
        try:
            parser.Parse(data, True)
        except expat.ExpatError as error:
            raise WorkbenchError('BAD_XML', str(error)) from error
        return tokens, empty_control_indexes

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
        self.hwp_sections = {}
        self.fonts = []
        expanded_total = 0
        section_names = sorted(
            (name for name in self.parts if re.fullmatch(r'BodyText/Section\d+', name)),
            key=lambda name: int(name.rsplit('Section', 1)[1]),
        )
        for section_index, name in enumerate(section_names):
            raw = self._hwp_stream(name)
            expanded_total += len(raw)
            require(expanded_total <= MAX_EXPANDED, 'LIMIT', 'Expanded HWP sections exceed limit.')
            self.sections[name] = raw
            self._read_hwp_section(name, raw, section_index)
        require(bool(self.sections), 'NOT_HWP', 'No HWP body sections found.')
        self.warn('HWP_REFLOW_UNVERIFIED', 'warning', 'Text length may change in existing plain text runs. Stored line starts are adjusted, but layout is stale; readers must recompute line breaks. Hancom reflow has not been verified.')
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

    def _read_hwp_section(self, name, raw, section_index):
        section = parse_section(raw)
        self.hwp_sections[name] = section
        paragraph = 0
        children = []
        top_paragraph = -1
        paragraph_level = 0
        controls = []
        for item in section.records:
            record = item.index
            tag = item.tag
            level = item.level
            size = len(item.data)
            start = item.payload_start
            end = start + size
            while controls and level <= controls[-1]['level']:
                controls.pop()
            if tag == 71 and size >= 4:
                controls.append({'level': level, 'id': raw[start:start + 4][::-1],
                                 'tableSeen': False, 'listRole': None})
            if controls and controls[-1]['id'] == b'tbl ':
                table = controls[-1]
                # Direct TABLE children divide caption lists from cell lists.
                # Nested tables have their own control stack entry.
                if level == table['level'] + 1:
                    if tag == 77:
                        table['tableSeen'] = True
                        table['listRole'] = None
                    elif tag == 72:
                        table['listRole'] = 'cell' if table['tableSeen'] else 'caption'
            if tag == 66:
                paragraph += 1
                paragraph_level = level
                if level == 0:
                    top_paragraph += 1
            if tag == 67:
                require(size % 2 == 0, 'BAD_HWP_TEXT', 'Misaligned UTF-16 text record.')
                # Edit only contiguous printable text; native controls stay opaque.
                offset = start
                segment = 0
                char_offset = 0
                control = controls[-1] if controls else None
                in_cell = bool(control and control['id'] == b'tbl ' and control['listRole'] == 'cell')
                nested = paragraph_level > 0
                role = 'text-run'
                if nested:
                    role = 'table-cell' if in_cell else 'text-box'
                    if control and control['id'] == b'tbl ' and control['listRole'] == 'caption':
                        role = 'table-caption'
                    if any(item['id'] in {b'head', b'foot'} for item in controls):
                        role = 'header-footer'
                def append_text(begin, finish, text):
                    node_id = f'{name}#record:{record}:text:{segment}'
                    anchor = {'section': section_index, 'paragraph': max(0, top_paragraph),
                              'charOffset': None if nested else char_offset, 'inCell': in_cell}
                    paragraph_record = section.owners.get(record)
                    text_start = section.text_offsets.get(record, 0) + (begin - start) // 2
                    text_end = text_start + (finish - begin) // 2
                    self.nodes.append(TextNode(
                        node_id, text, name, role, True, paragraph, {}, anchor,
                        paragraph_record, text_start, text_end,
                    ))
                    self.positions[node_id] = {
                        'start': begin, 'end': finish, 'part': name,
                        'record': record, 'paragraphRecordIndex': paragraph_record,
                        'textStart': text_start, 'textEnd': text_end,
                    }
                    children.append({'type': 'paragraph', 'children': [{'type': 'text', 'node': node_id}]})

                while offset < end:
                    code = struct.unpack_from('<H', raw, offset)[0]
                    if code < 32:
                        width = 16 if (1 <= code <= 9 or 11 <= code <= 12 or 14 <= code <= 23) else 2
                        require(offset + width <= end, 'BAD_HWP_TEXT', 'Truncated HWP control sequence.')
                        # rhwp elides extended slots except auto-number. Tabs,
                        # line breaks and selected single-unit controls each
                        # contribute one scalar; the paragraph terminator ends text.
                        if code in {9, 10, 18, 24, 25, 30, 31}:
                            char_offset += 1
                        if code == 13:
                            if segment == 0:
                                append_text(offset, offset, '')
                            break
                        offset += width
                        continue
                    begin = offset
                    while offset < end and struct.unpack_from('<H', raw, offset)[0] >= 32:
                        offset += 2
                    try:
                        text = raw[begin:offset].decode('utf-16le')
                    except UnicodeError:
                        raise WorkbenchError('BAD_HWP_TEXT', 'Invalid UTF-16 in paragraph text.')
                    append_text(begin, offset, text)
                    char_offset += len(text)
                    segment += 1
        self.view.append({'type': 'section', 'label': name, 'children': children})

    def inspect(self):
        return dict(schemaVersion=1,format=self.format,name=self.name,sha256=self.hash,
                    nodes=[asdict(n)|{'text':self.edits.get(n.id,n.text)} for n in self.nodes],
                    warnings=self.warnings,view=self.view,fonts=self.fonts,
                    capabilities={'textEdit':not self.block_save,'lengthChangingText':not self.block_save,
                                  'structuralEdit':False,'formattingEdit':False,'certifiedLayout':False,'saveAs':not self.block_save},
                    parts=[{'name':k,'size':len(v),'sha256':sha(v)} for k,v in self.parts.items()],
                    pendingChanges=len(self.edits))

    def _references(self) -> dict[str, str]:
        references = {}
        section_numbers = {}
        section_counts = {}
        for node in self.nodes:
            section = section_numbers.setdefault(node.part, len(section_numbers))
            index = section_counts.get(node.part, 0)
            section_counts[node.part] = index + 1
            references[f's{section}:t{index}'] = node.id
        return references

    def resolve_node_id(self, node_id: str) -> str:
        require(isinstance(node_id, str), 'BAD_ARGUMENTS', 'nodeId must be a full ID or short reference.')
        resolved = self.references.get(node_id, node_id)
        require(resolved in self.by_id, 'NODE_NOT_FOUND', 'Unknown node ID: ' + node_id)
        return resolved

    def patch(self, patch: dict):
        require(isinstance(patch, dict) and patch.get('schemaVersion') == 1,
                'BAD_PATCH', 'Expected patch schemaVersion 1.')
        require(patch.get('baseSha256') == self.hash, 'STALE_DOCUMENT',
                'Patch hash differs from source document; re-inspect before editing.')
        operations = patch.get('operations')
        require(isinstance(operations, list) and 0 < len(operations) <= 1000,
                'BAD_PATCH', 'Supply 1–1000 operations.')
        require(not self.block_save, 'SAVE_BLOCKED', 'Protected/active content blocks edits.')
        pending = dict(self.edits)
        seen = set()
        for operation in operations:
            require(isinstance(operation, dict) and set(operation) == {'op', 'nodeId', 'expectedText', 'text'},
                    'BAD_PATCH', 'Each operation requires exactly op, nodeId, expectedText, text.')
            require(operation['op'] == 'set_text' and isinstance(operation['nodeId'], str),
                    'BAD_PATCH', 'Only set_text is implemented.')
            node_id = self.resolve_node_id(operation['nodeId'])
            require(node_id not in seen, 'BAD_PATCH', 'Duplicate target in patch.')
            seen.add(node_id)
            node = self.by_id[node_id]
            require(node.editable, 'UNSUPPORTED_ELEMENT', 'This text is not safely editable.')
            require(operation['expectedText'] == pending.get(node_id, node.text),
                    'STALE_TEXT', 'Expected text no longer matches ' + node_id)
            value = operation['text']
            if self.format == 'hwp' and isinstance(value, str):
                require(all(ord(char) >= 32 for char in value), 'UNSUPPORTED_ELEMENT',
                        'Plain text only: do not touch native controls, tabs, fields or paragraph breaks.')
            check_text(value)
            if self.format == 'hwp':
                validate_text_edit(self.hwp_sections[node.part], self.positions[node_id])
            if value == node.text:
                pending.pop(node_id, None)
            else:
                pending[node_id] = value
        # All-or-nothing; no mutations until every operation has passed.
        self.edits = pending
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
        expected_sections = {}
        by_part = {}
        for node_id, value in self.edits.items():
            by_part.setdefault(self.by_id[node_id].part, []).append((node_id, value))
        for part, items in by_part.items():
            raw = self.parts[part] if self.format == 'hwpx' else self.sections[part]
            if self.format == 'hwp':
                native_edits = [(self.positions[node_id], value.encode('utf-16le'))
                                for node_id, value in items]
                raw = rewrite_section(self.hwp_sections[part], native_edits)
                expected_sections[part] = raw
            else:
                replacements = []
                for node_id, value in items:
                    position = self.positions[node_id]
                    if position['selfclose']:
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
        if self.format == 'hwp':
            # Deleting a run can renumber later text segments in this revision.
            # Verify complete native sections, including controls and metadata,
            # rather than binding reopened nodes to the old revision's IDs.
            for part, expected in expected_sections.items():
                require(check.sections[part] == expected, 'VERIFY_FAILED', 'Edited HWP section differs on reopen: ' + part)
        else:
            for node_id, text in self.edits.items():
                require(node_id in check.by_id and check.by_id[node_id].text == text, 'VERIFY_FAILED', 'Edit was not preserved on reopen: ' + node_id)
        return result
