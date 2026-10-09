"""Self-authored synthetic format fixtures, not Hancom-certified documents."""
import io,struct,zipfile,zlib
from hwp_workbench.cfb import CompoundFile,MAGIC,FREE,END,p32,p64
from hwp_workbench.document import HP,HH,HC

# HWPUNIT is 1/7200 inch. These values describe an A4 page with 30 mm
# side margins; no font files or third-party document bytes are included.
PAGE_WIDTH = 59528
PAGE_HEIGHT = 84188
PAGE_SIDE_MARGIN = 8504
TEXT_WIDTH = PAGE_WIDTH - 2 * PAGE_SIDE_MARGIN
FONT_NAME = 'Apple SD Gothic Neo'


def hwpx_paragraph(text, paragraph_id=0, style=0, prefix=''):
    return (
        f'<hp:p id="{paragraph_id}" paraPrIDRef="0" styleIDRef="0" pageBreak="0" columnBreak="0" merged="0">'
        f'<hp:run charPrIDRef="{style}">{prefix}<hp:t>{text}</hp:t></hp:run></hp:p>'
    )


def hwpx_section():
    rows = []
    values = [
        ['항목 / Item', '내용 / Details'],
        ['기관명 / Organisation', '마리나체인'],
        ['담당자 / Contact', 'Jonathan Jie'],
        ['상태 / Status', '검토 중'],
    ]
    for row, cells in enumerate(values):
        authored_cells = []
        for column, value in enumerate(cells):
            paragraph = hwpx_paragraph(value, row * 2 + column + 3, 1 if row == 0 else 0)
            authored_cells.append(
                '<hp:tc borderFillIDRef="1">'
                f'<hp:subList id="" textDirection="HORIZONTAL" lineWrap="BREAK" vertAlign="CENTER">{paragraph}</hp:subList>'
                f'<hp:cellAddr colAddr="{column}" rowAddr="{row}"/>'
                '<hp:cellSpan colSpan="1" rowSpan="1"/>'
                '<hp:cellSz width="21260" height="2800"/>'
                '<hp:cellMargin left="141" right="141" top="141" bottom="141"/>'
                '</hp:tc>'
            )
        rows.append('<hp:tr>' + ''.join(authored_cells) + '</hp:tr>')
    page = (
        '<hp:secPr textDirection="HORIZONTAL" spaceColumns="0" tabStop="8000" masterPageCnt="0">'
        f'<hp:pagePr landscape="NARROWLY" width="{PAGE_WIDTH}" height="{PAGE_HEIGHT}" gutterType="LEFT_ONLY">'
        f'<hp:margin left="{PAGE_SIDE_MARGIN}" right="{PAGE_SIDE_MARGIN}" top="5669" bottom="4252" header="4252" footer="4252" gutter="0"/>'
        '</hp:pagePr></hp:secPr>'
        '<hp:ctrl><hp:colPr id="" type="NEWSPAPER" layout="LEFT" colCount="1" sameSz="1" sameGap="0"/></hp:ctrl>'
    )
    paragraphs = [
        hwpx_paragraph('내부 업무 검토서', 0, 2, page),
        hwpx_paragraph('INTERNAL REVIEW · SYNTHETIC TEST DOCUMENT', 1),
        hwpx_paragraph('01  기본 정보 / Basic information', 2, 1),
        '<hp:p id="40" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0">'
        '<hp:tbl id="4" rowCnt="4" colCnt="2" cellSpacing="0" borderFillIDRef="1" pageBreak="CELL" repeatHeader="1">'
        '<hp:sz width="42520" height="11200" widthRelTo="ABSOLUTE" heightRelTo="ABSOLUTE" protect="0"/>'
        '<hp:pos treatAsChar="1" affectLSpacing="0" flowWithText="1" allowOverlap="0" holdAnchorAndSO="0" vertRelTo="PARA" horzRelTo="COLUMN" vertAlign="TOP" horzAlign="LEFT" vertOffset="0" horzOffset="0"/>'
        '<hp:outMargin left="0" right="0" top="0" bottom="0"/>'
        '<hp:inMargin left="141" right="141" top="141" bottom="141"/>'
        + ''.join(rows) + '</hp:tbl></hp:run></hp:p>',
        hwpx_paragraph('02  검토 의견 / Review notes', 50, 1),
        hwpx_paragraph('공식 문서의 원본 구조를 유지하면서 필요한 텍스트만 수정합니다.', 51),
        hwpx_paragraph('This is an authored test fixture, not an official government form.', 52),
        hwpx_paragraph('03  보존 확인 / Preservation checks', 53, 1),
        hwpx_paragraph('표 구조, 글자 속성, 알 수 없는 데이터는 원본 패키지에 유지됩니다.', 54),
        '<hp:p id="55" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:t/></hp:run></hp:p>',
        # Deliberately unsupported object marker tests opaque retention/warnings.
        '<hp:p id="56" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:pic id="99"/></hp:run></hp:p>',
    ]
    return (
        '<?xml version="1.0" encoding="UTF-8"?>'
        f'<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="{HP}" xmlns:hc="{HC}">'
        + ''.join(paragraphs) + '</hs:sec>'
    )


def hwpx_header():
    languages = ['HANGUL', 'LATIN', 'HANJA', 'JAPANESE', 'OTHER', 'SYMBOL', 'USER']
    fonts = ''.join(
        f'<hh:fontface lang="{language}" fontCnt="1"><hh:font id="0" face="{FONT_NAME}" type="TTF" isEmbedded="0"/></hh:fontface>'
        for language in languages
    )
    characters = []
    for index, height in enumerate([1000, 1100, 2200]):
        bold = '<hh:bold/>' if index else ''
        characters.append(
            f'<hh:charPr id="{index}" height="{height}" textColor="#000000" shadeColor="none" useFontSpace="0" useKerning="0">'
            '<hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>'
            '<hh:ratio hangul="100" latin="100" hanja="100" japanese="100" other="100" symbol="100" user="100"/>'
            '<hh:spacing hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>'
            '<hh:relSz hangul="100" latin="100" hanja="100" japanese="100" other="100" symbol="100" user="100"/>'
            '<hh:offset hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>'
            + bold + '</hh:charPr>'
        )
    return (
        f'<hh:head xmlns:hh="{HH}" xmlns:hc="{HC}" version="1.5" secCnt="1">'
        '<hh:beginNum page="1" footnote="1" endnote="1" pic="1" tbl="1" equation="1"/>'
        f'<hh:refList><hh:fontfaces itemCnt="7">{fonts}</hh:fontfaces>'
        '<hh:borderFills itemCnt="1"><hh:borderFill id="1" threeD="0" shadow="0" centerLine="NONE">'
        '<hh:leftBorder type="SOLID" width="0.1 mm" color="#000000"/>'
        '<hh:rightBorder type="SOLID" width="0.1 mm" color="#000000"/>'
        '<hh:topBorder type="SOLID" width="0.1 mm" color="#000000"/>'
        '<hh:bottomBorder type="SOLID" width="0.1 mm" color="#000000"/>'
        '</hh:borderFill></hh:borderFills>'
        '<hh:charProperties itemCnt="3">' + ''.join(characters) + '</hh:charProperties>'
        '<hh:tabProperties itemCnt="1"><hh:tabPr id="0" autoTabLeft="0" autoTabRight="0"/></hh:tabProperties>'
        '<hh:paraProperties itemCnt="1"><hh:paraPr id="0" tabPrIDRef="0" condense="0" fontLineHeight="0" snapToGrid="1">'
        '<hh:align horizontal="LEFT" vertical="BASELINE"/>'
        '<hh:heading type="NONE" idRef="0" level="0"/>'
        '<hh:lineSpacing type="PERCENT" value="160" unit="HWPUNIT"/>'
        '</hh:paraPr></hh:paraProperties>'
        '<hh:styles itemCnt="1"><hh:style id="0" type="PARA" name="Normal" engName="Normal" paraPrIDRef="0" charPrIDRef="0" nextStyleIDRef="0" langID="1042" lockForm="0"/></hh:styles>'
        '</hh:refList></hh:head>'
    )


def make_hwpx(extra=None, section=None):
    if section is None:
        section = hwpx_section()
    # rhwp resolves manifest hrefs from the ZIP root, not Contents/.
    manifest = (
        '<opf:package xmlns:opf="http://www.idpf.org/2007/opf/" version="3.0">'
        '<opf:manifest><opf:item id="header" href="Contents/header.xml" media-type="application/xml"/>'
        '<opf:item id="section0" href="Contents/section0.xml" media-type="application/xml"/></opf:manifest>'
        '<opf:spine><opf:itemref idref="section0"/></opf:spine></opf:package>'
    )
    parts = {
        'mimetype': b'application/hwp+zip',
        'version.xml': b'<HCFVersion major="5" minor="0"/>',
        'META-INF/container.xml': b'<container><rootfiles><rootfile full-path="Contents/content.hpf"/></rootfiles></container>',
        'Contents/content.hpf': manifest.encode(),
        'Contents/header.xml': hwpx_header().encode(),
        'Contents/section0.xml': section.encode(),
        'BinData/opaque.dat': b'OPAQUE-PAYLOAD-DO-NOT-ALTER\x00\xff\x01',
        'Preview/PrvText.txt': '내부 업무 검토서'.encode('utf-16le'),
    }
    parts.update(extra or {})
    output = io.BytesIO()
    with zipfile.ZipFile(output, 'w') as archive:
        for name, data in parts.items():
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED if name == 'mimetype' else zipfile.ZIP_DEFLATED
            archive.writestr(info, data)
    return output.getvalue()

def record(tag,data,level=0):
    if len(data)<4095:return struct.pack('<I',tag|(level<<10)|(len(data)<<20))+data
    return struct.pack('<II',tag|(level<<10)|(4095<<20),len(data))+data

def cfb_from_streams(streams,major=3):
    # Build valid directory topology independently of the reader. The production
    # rewriter is reused to allocate sectors; tests separately examine offsets.
    ss=512 if major==3 else 4096
    paths=set(streams)
    for p in list(paths):
        bits=p.split('/')
        for n in range(1,len(bits)):paths.add('/'.join(bits[:n]))
    ordered=sorted(paths)
    ids={p:i+1 for i,p in enumerate(ordered)}
    entries=[bytearray(128) for _ in range(len(ordered)+1)]
    def init(e,name,kind):
        nm=(name+'\0').encode('utf-16le');e[:len(nm)]=nm;struct.pack_into('<H',e,64,len(nm));e[66]=kind;e[67]=1
        for off in [68,72,76]:p32(e,off,FREE)
        p32(e,116,END)
    init(entries[0],'Root Entry',5)
    for path,idx in ids.items():init(entries[idx],path.split('/')[-1],2 if path in streams else 1)
    parents=['']+[p for p in ordered if p not in streams]
    for parent in parents:
        children=[p for p in ordered if p.rpartition('/')[0]==parent]
        children.sort(key=lambda p:(len(p.split('/')[-1].encode('utf-16le')),p.split('/')[-1].upper()))
        maxdepth=(len(children)+1).bit_length()-2
        def tree(items,depth=0):
            if not items:return FREE
            m=len(items)//2;idx=ids[items[m]];e=entries[idx]
            p32(e,68,tree(items[:m],depth+1));p32(e,72,tree(items[m+1:],depth+1))
            # Leaves at the deepest level can be red in a near-complete tree.
            if depth>maxdepth:e[67]=0
            return idx
        p32(entries[ids[parent] if parent else 0],76,tree(children))
    directory=b''.join(entries).ljust(((len(entries)*128+ss-1)//ss)*ss,b'\0')
    header=bytearray(ss);header[:8]=MAGIC;struct.pack_into('<HHHHH',header,24,0x3e,major,0xfffe,9 if major==3 else 12,6);p32(header,56,4096)
    header[76:512]=b'\xff'*436
    obj=CompoundFile.__new__(CompoundFile);obj.major=major;obj.ss=ss;obj.data=bytes(header);obj.directory=directory;obj.paths=ids;obj.streams={p:b'' for p in streams}
    return obj.rewrite(streams)

def hwp_string(value):
    payload = value.encode('utf-16le')
    return struct.pack('<H', len(payload) // 2) + payload


def hwp_doc_info():
    # DocInfo records: document properties, ID mappings, seven font categories,
    # character shape, tab definition, paragraph shape and paragraph style.
    properties = struct.pack('<7H3I', 1, 1, 1, 1, 1, 1, 1, 0, 0, 0)
    counts = [0] + [1] * 7 + [0, 1, 1, 0, 0, 1, 1, 0]
    mappings = struct.pack('<16I', *counts)
    font = b'\0' + hwp_string(FONT_NAME)
    character_shape = struct.pack('<7H', *([0] * 7))
    character_shape += bytes([100] * 7 + [0] * 7 + [100] * 7 + [0] * 7)
    character_shape += struct.pack('<iIbb4IHI', 1000, 0, 0, 0, 0, 0, 0xffffffff, 0, 0, 0)
    # Left alignment is bit 2. Line spacing 160 means 160 percent.
    paragraph_shape = struct.pack('<I6i3H4h3I', 1 << 2, 0, 0, 0, 0, 0, 160, 0, 0, 0, 0, 0, 0, 0, 0, 0, 160)
    style = hwp_string('Normal') + hwp_string('Normal') + struct.pack('<BBhHHH', 0, 0, 1042, 0, 0, 0)
    return (
        record(16, properties)
        + record(17, mappings)
        + b''.join(record(19, font, 1) for _ in range(7))
        + record(21, character_shape, 1)
        + record(22, struct.pack('<II', 0, 0), 1)
        + record(25, paragraph_shape, 1)
        + record(26, style, 1)
    )


def raw_deflate(payload):
    compressor = zlib.compressobj(wbits=-15)
    return compressor.compress(payload) + compressor.flush()


def make_hwp(compressed=True, flags=0, major=3, long=False):
    header = bytearray(256)
    header[:17] = b'HWP Document File'
    header[32:36] = bytes([0, 3, 0, 5])
    p32(header, 36, int(compressed) | flags)
    text = '마리나체인 검토 중' if not long else '테스트 문서 내용 ' * 900
    # Each extended control occupies eight UTF-16 code units. Matching CTRL_HEADER
    # records below provide section and column definitions; controls are not text.
    section_control = struct.pack('<H', 2) + b'dces' + b'\0' * 8 + struct.pack('<H', 2)
    column_control = struct.pack('<H', 2) + b'dloc' + b'\0' * 8 + struct.pack('<H', 2)
    paragraph_text = section_control + column_control + text.encode('utf-16le') + b'\r\0'
    line_starts = list(range(0, len(text), 80))
    line_segments = b''.join(
        struct.pack('<I7iI', 0 if index == 0 else start + 16, index * 1600, 1000, 1000, 850, 600, 0, TEXT_WIDTH, 0x00060000)
        for index, start in enumerate(line_starts)
    )
    # PARA_HEADER has character count, control mask, style IDs, record counts,
    # and instance ID. The high bit marks the last paragraph in the list.
    paragraph_header = struct.pack('<IIHBBHHHI', len(paragraph_text) // 2 | (1 << 31), 1 << 2, 0, 0, 0, 1, 0, len(line_starts), 1)
    section_definition = b'dces' + struct.pack('<IhhhI5H', 0, 0, 0, 0, 8000, 0, 1, 1, 1, 1)
    page_definition = struct.pack('<10I', PAGE_WIDTH, PAGE_HEIGHT, PAGE_SIDE_MARGIN, PAGE_SIDE_MARGIN, 5669, 4252, 4252, 4252, 0, 0)
    # Column count = 1 (bits 2-9); same-sized columns = bit 12.
    column_definition = b'dloc' + struct.pack('<3HBBI', 0x1004, 0, 0, 0, 0, 0)
    body = (
        record(66, paragraph_header)
        + record(67, paragraph_text, 1)
        + record(68, struct.pack('<II', 0, 0), 1)
        + record(69, line_segments, 1)
        + record(71, section_definition, 1)
        + record(73, page_definition, 2)
        + record(71, column_definition, 1)
    )
    doc_info = hwp_doc_info()
    if compressed:
        body = raw_deflate(body)
        doc_info = raw_deflate(doc_info)
    streams = {
        'FileHeader': bytes(header),
        'DocInfo': doc_info,
        'BodyText/Section0': body,
        'PrvText': text.encode('utf-16le'),
        'BinData/opaque.bin': b'IMAGE-OPAQUE' * 800,
    }
    return cfb_from_streams(streams, major)
