"""Self-authored synthetic format fixtures, not Hancom-certified documents."""
import io,struct,zipfile,zlib
from hwp_workbench.cfb import CompoundFile,MAGIC,FREE,END,p32,p64
from hwp_workbench.document import HP,HH,HC

def make_hwpx(extra=None,section=None):
    if section is None:
        def p(t,n=0,style=0):return f'<hp:p id="{n}" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="{style}"><hp:t>{t}</hp:t></hp:run></hp:p>'
        rows=''
        for r,vals in enumerate([['항목 / Item','내용 / Details'],['기관명 / Organisation','마리나체인'],['담당자 / Contact','Jonathan Jie'],['상태 / Status','검토 중']]):
            rows+='<hp:tr>'+''.join(f'<hp:tc><hp:subList>{p(v,r*2+c+3,1 if r==0 else 0)}</hp:subList><hp:cellAddr colAddr="{c}" rowAddr="{r}"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="22000" height="2800"/></hp:tc>' for c,v in enumerate(vals))+'</hp:tr>'
        section=f'''<?xml version="1.0" encoding="UTF-8"?><hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="{HP}" xmlns:hc="{HC}">{p('내부 업무 검토서',0,2)}{p('INTERNAL REVIEW · SYNTHETIC TEST DOCUMENT',1)}{p('01  기본 정보 / Basic information',2,1)}<hp:p id="40"><hp:run><hp:tbl id="4" rowCnt="4" colCnt="2">{rows}</hp:tbl></hp:run></hp:p>{p('02  검토 의견 / Review notes',50,1)}{p('공식 문서의 원본 구조를 유지하면서 필요한 텍스트만 수정합니다.',51)}{p('This is an authored test fixture, not an official government form.',52)}{p('03  보존 확인 / Preservation checks',53,1)}{p('표 구조, 글자 속성, 알 수 없는 데이터는 원본 패키지에 유지됩니다.',54)}<hp:p id="55"><hp:run><hp:t/></hp:run></hp:p><hp:p id="56"><hp:run><hp:pic id="99"/></hp:run></hp:p></hs:sec>'''
    parts={
        'mimetype':b'application/hwp+zip',
        'version.xml':b'<HCFVersion major="5" minor="0"/>',
        'META-INF/container.xml':b'<container><rootfiles><rootfile full-path="Contents/content.hpf"/></rootfiles></container>',
        'Contents/content.hpf':b'<package><manifest><item id="section0" href="section0.xml" media-type="application/xml"/></manifest><spine><itemref idref="section0"/></spine></package>',
        'Contents/header.xml':f'<hh:head xmlns:hh="{HH}"><hh:refList><hh:fontfaces><hh:fontface><hh:font id="0" face="Apple SD Gothic Neo"/></hh:fontface></hh:fontfaces><hh:charProperties><hh:charPr id="0" height="1000"/><hh:charPr id="1" height="1100"><hh:bold/></hh:charPr><hh:charPr id="2" height="2200"><hh:bold/></hh:charPr></hh:charProperties></hh:refList></hh:head>'.encode(),
        'Contents/section0.xml':section.encode(),
        'BinData/opaque.dat':b'OPAQUE-PAYLOAD-DO-NOT-ALTER\x00\xff\x01',
        'Preview/PrvText.txt':'내부 업무 검토서'.encode('utf-16le')}
    parts.update(extra or {})
    output=io.BytesIO()
    with zipfile.ZipFile(output,'w') as z:
        for name,data in parts.items():
            info=zipfile.ZipInfo(name,date_time=(2026,1,1,0,0,0));info.compress_type=zipfile.ZIP_STORED if name=='mimetype' else zipfile.ZIP_DEFLATED
            z.writestr(info,data)
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

def make_hwp(compressed=True,flags=0,major=3,long=False):
    header=bytearray(256);header[:17]=b'HWP Document File';header[32:36]=bytes([7,1,0,5]);p32(header,36,int(compressed)|flags)
    text='마리나체인 검토 중' if not long else '테스트 문서 내용 '*900
    ctrl=bytes([2,0])+b'dces'+b'\0'*8+bytes([2,0])
    raw=record(66,struct.pack('<I',len(text)+9)+b'\0'*18)+record(67,ctrl+text.encode('utf-16le')+b'\r\0',1)+record(68,struct.pack('<II',0,0),1)+record(69,b'LINE-SEGMENT-OPAQUE-DATA',1)
    if compressed:
        c=zlib.compressobj(wbits=-15);body=c.compress(raw)+c.flush()
    else:body=raw
    streams={'FileHeader':bytes(header),'DocInfo':b'DOCINFO-OPAQUE','BodyText/Section0':body,'PrvText':text.encode('utf-16le'),'BinData/opaque.bin':b'IMAGE-OPAQUE'*800}
    return cfb_from_streams(streams,major)
