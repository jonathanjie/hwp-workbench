"""Self-authored OOXML fixture; no external files, fonts or document samples."""
import io
import struct
import zlib
import zipfile

W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
R = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'


def png():
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>2I5B', 1, 1, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(b'\0\x20\x80\xc0')) + chunk(b'IEND', b''))


def package(parts):
    output = io.BytesIO()
    with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in parts.items():
            # Source hash checks compare fixture bytes across multiple calls.
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, data)
    return output.getvalue()


def docx(extra_body='', extra_parts=None):
    border = '<w:tcBorders><w:top w:val="single" w:sz="8" w:color="123456"/><w:bottom w:val="double" w:sz="12" w:color="334455"/></w:tcBorders>'
    body = '''<w:p><w:pPr><w:pStyle w:val="Heading1"/><w:jc w:val="center"/></w:pPr><w:r><w:t>Heading 한글</w:t></w:r></w:p>
    <w:p><w:pPr><w:ind w:left="360" w:right="180" w:firstLine="120"/><w:spacing w:before="120" w:after="160" w:line="300" w:lineRule="auto"/></w:pPr>
    <w:r><w:rPr><w:b/><w:i/><w:u w:val="single"/><w:strike/><w:sz w:val="28"/><w:color w:val="2468AC"/><w:rFonts w:ascii="Arial" w:eastAsia="Arial"/></w:rPr><w:t xml:space="preserve">Styled text </w:t><w:br/><w:t>second line</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>Bullet item</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>Number item</w:t></w:r></w:p>'''
    body += '<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="1800"/><w:gridCol w:w="2200"/></w:tblGrid><w:tr>'
    body += '<w:tc><w:tcPr><w:gridSpan w:val="2"/><w:shd w:fill="DDEEFF"/>' + border + '</w:tcPr><w:p><w:r><w:t>Across columns</w:t></w:r></w:p></w:tc></w:tr>'
    body += '<w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/>' + border + '</w:tcPr><w:p><w:r><w:t>Across rows</w:t></w:r></w:p></w:tc><w:tc><w:tcPr/><w:p><w:r><w:t>Right one</w:t></w:r></w:p></w:tc></w:tr>'
    body += '<w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc><w:tcPr/><w:p><w:r><w:t>Right two</w:t></w:r></w:p></w:tc></w:tr></w:tbl>'
    body += '''<w:p><w:r><w:t>Image caption</w:t><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="457200"/><wp:docPr id="1" name="Authored pixel"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:blipFill><a:blip r:embed="image1"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>
    <w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>New page</w:t></w:r></w:p><w:p/>'''
    body += extra_body + '<w:sectPr><w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/><w:pgMar w:top="900" w:bottom="1000" w:left="1100" w:right="1200" w:header="400" w:footer="500" w:gutter="0"/></w:sectPr>'
    parts = {
        '[Content_Types].xml': '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>',
        '_rels/.rels': f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>',
        'word/document.xml': f'<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}</w:body></w:document>',
        'word/styles.xml': f'<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="36"/></w:rPr></w:style></w:styles>',
        'word/numbering.xml': f'<w:numbering xmlns:w="{W}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="3"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>',
        'word/_rels/document.xml.rels': f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="image1" Type="{R}/image" Target="media/pixel.png"/></Relationships>',
        'word/media/pixel.png': png(),
    }
    parts.update(extra_parts or {})
    return package(parts)
