"""Emit supported rich content as native HWPX; no HTML import is involved."""
from xml.etree import ElementTree as ET

from ..document import HP, HC, HH
from .model import Paragraph, Table, paragraphs
from .package import element, encoded, write_package

HS = 'http://www.hancom.co.kr/hwpml/2011/section'
OPF = 'http://www.idpf.org/2007/opf/'
LANGUAGES = ['HANGUL', 'LATIN', 'HANJA', 'JAPANESE', 'OTHER', 'SYMBOL', 'USER']
FONT_KEYS = ['hangul', 'latin', 'hanja', 'japanese', 'other', 'symbol', 'user']
# HWPUNIT is 1/7200 inch, five times the model's twip unit.
HWP_PER_TWIP = 5
# Border widths in DOCX are eighths of a point; one point is 25.4/72 mm.
MM_PER_BORDER_UNIT = 25.4 / 72 / 8
BORDER_TYPES = {'single': 'SOLID', 'double': 'DOUBLE_SLIM', 'dotted': 'DOT',
                'dashed': 'DASH', 'nil': 'NONE', 'none': 'NONE'}
UNDERLINE_TYPES = {'single': 'SOLID', 'double': 'DOUBLE', 'dotted': 'DOT', 'dash': 'DASH', 'wave': 'WAVE'}

for prefix, namespace in [('hp', HP), ('hc', HC), ('hh', HH), ('hs', HS), ('opf', OPF)]:
    ET.register_namespace(prefix, namespace)


class Writer:
    def __init__(self, document):
        self.document = document
        self.char_styles = [{}]
        self.para_styles = [{}]
        self.fonts = ['Arial']
        self.fills = [({}, None)]
        self.numbers = []
        self.parts = {'mimetype': b'application/hwp+zip'}
        self.object_id = 1
        self.paragraph_id = 0

    @staticmethod
    def _index(items, item):
        if item not in items:
            items.append(item)
        return items.index(item)

    def _fill_id(self, borders, shading):
        return self._index(self.fills, (borders, shading)) + 1

    def _paragraph(self, paragraph, prefix=None, *, container_only=False):
        para_style = dict(paragraph.style)
        if paragraph.numbering:
            para_style['numbering'] = paragraph.numbering
            self._index(self.numbers, paragraph.numbering)
        para_id = self._index(self.para_styles, para_style)
        root = ET.Element('{' + HP + '}p', dict(id=str(self.paragraph_id), paraPrIDRef=str(para_id),
                          styleIDRef=str(paragraph.heading), pageBreak=str(int(paragraph.page_break)), columnBreak='0', merged='0'))
        self.paragraph_id += 1
        if prefix is not None:
            root.append(prefix)
        for run in paragraph.runs:
            char_id = self._index(self.char_styles, dict(run.style))
            self._index(self.fonts, run.style.get('font', 'Arial'))
            native_run = element(root, HP, 'run', charPrIDRef=char_id)
            if run.image:
                self._image(native_run, run.image)
            if run.text or not run.image:
                text = element(native_run, HP, 't')
                pieces = run.text.replace('\r\n', '\n').replace('\r', '\n').split('\n')
                tail = None
                for index, piece in enumerate(pieces):
                    if index:
                        tail = element(text, HP, 'lineBreak')
                    tabs = piece.split('\t')
                    for tab_index, content in enumerate(tabs):
                        if tab_index:
                            tail = element(text, HP, 'tab', width=4000, leader='0', type='1')
                        if tail is None:
                            text.text = (text.text or '') + content
                        else:
                            tail.tail = (tail.tail or '') + content
        if not paragraph.runs and not container_only:
            element(element(root, HP, 'run', charPrIDRef=0), HP, 't')
        return root

    def _object_common(self, root, width, height):
        element(root, HP, 'sz', width=width, height=height, widthRelTo='ABSOLUTE', heightRelTo='ABSOLUTE', protect=0)
        element(root, HP, 'pos', treatAsChar=1, affectLSpacing=0, flowWithText=1, allowOverlap=0,
                holdAnchorAndSO=0, vertRelTo='PARA', horzRelTo='COLUMN', vertAlign='TOP', horzAlign='LEFT', vertOffset=0, horzOffset=0)
        element(root, HP, 'outMargin', left=0, right=0, top=0, bottom=0)

    def _image(self, parent, image):
        index = self.object_id
        self.object_id += 1
        extension = {'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/bmp': 'bmp'}[image.media_type]
        part = f'BinData/image{index}.{extension}'
        self.parts[part] = image.data
        width, height = image.width * HWP_PER_TWIP, image.height * HWP_PER_TWIP
        picture = element(parent, HP, 'pic', id=index, zOrder=0, numberingType='PICTURE', textWrap='TOP_AND_BOTTOM', textFlow='BOTH_SIDES', reverse=0)
        offset = element(picture, HP, 'offset', x=0, y=0)
        element(picture, HP, 'orgSz', width=width, height=height)
        element(picture, HP, 'curSz', width=width, height=height)
        element(picture, HP, 'flip', horizontal=0, vertical=0)
        element(picture, HP, 'rotationInfo', angle=0, centerX=width // 2, centerY=height // 2, rotateimage=1)
        rendering = element(picture, HP, 'renderingInfo')
        for tag in ['transMatrix', 'scaMatrix', 'rotMatrix']:
            element(rendering, HC, tag, e1=1, e2=0, e3=0, e4=0, e5=1, e6=0)
        line = element(picture, HP, 'lineShape', color='#000000', width=0, style='NONE', endCap='FLAT', headStyle='NORMAL', tailStyle='NORMAL')
        rectangle = element(picture, HP, 'imgRect')
        for tag, x, y in [('pt0', 0, 0), ('pt1', width, 0), ('pt2', width, height), ('pt3', 0, height)]:
            element(rectangle, HC, tag, x=x, y=y)
        element(picture, HP, 'imgClip', left=0, right=width, top=0, bottom=height)
        element(picture, HP, 'inMargin', left=0, right=0, top=0, bottom=0)
        element(picture, HC, 'img', binaryItemIDRef=f'image{index}', bright=0, contrast=0, effect='REAL_PIC', alpha=0)
        self._object_common(picture, width, height)

    def _table(self, table, prefix=None):
        carrier = self._paragraph(Paragraph(), prefix, container_only=True)
        run = element(carrier, HP, 'run', charPrIDRef=0)
        table_id = self.object_id
        self.object_id += 1
        native = element(run, HP, 'tbl', id=table_id, rowCnt=len(table.rows), colCnt=len(table.widths),
                         cellSpacing=0, borderFillIDRef=1, pageBreak='CELL', repeatHeader=0)
        widths = [width * HWP_PER_TWIP for width in table.widths]
        self._object_common(native, sum(widths), max(1, len(table.rows)) * 2400)
        element(native, HP, 'inMargin', left=0, right=0, top=0, bottom=0)
        for row_index, row in enumerate(table.rows):
            tr = element(native, HP, 'tr')
            for cell in row:
                # HWPX has per-cell borders, so resolve DOCX inside borders here.
                borders = dict(cell.borders)
                for side, inside in [('top', 'insideH'), ('bottom', 'insideH'), ('left', 'insideV'), ('right', 'insideV')]:
                    if side not in borders and inside in borders:
                        borders[side] = borders[inside]
                tc = element(tr, HP, 'tc', name='', header=0, hasMargin=1, protect=0,
                             borderFillIDRef=self._fill_id(borders, cell.shading))
                sublist = element(tc, HP, 'subList', id='', textDirection='HORIZONTAL', lineWrap='BREAK', vertAlign='TOP',
                                  linkListIDRef=0, linkListNextIDRef=0, textWidth=0, textHeight=0, hasTextRef=0, hasNumRef=0)
                for block in cell.blocks or [Paragraph()]:
                    sublist.append(self._paragraph(block) if isinstance(block, Paragraph) else self._table(block))
                element(tc, HP, 'cellAddr', colAddr=cell.column, rowAddr=row_index)
                element(tc, HP, 'cellSpan', colSpan=cell.colspan, rowSpan=cell.rowspan)
                element(tc, HP, 'cellSz', width=sum(widths[cell.column:cell.column + cell.colspan]), height=2400 * cell.rowspan)
                element(tc, HP, 'cellMargin', left=0, right=0, top=0, bottom=0)
        return carrier

    def _section_prefix(self, page):
        run = ET.Element('{' + HP + '}run', {'charPrIDRef': '0'})
        section = element(run, HP, 'secPr', id='', textDirection='HORIZONTAL', spaceColumns=0, tabStop=4000, masterPageCnt=0)
        landscape = 'NARROWLY' if page['orientation'] == 'landscape' else 'WIDELY'
        # Native HWPX stores unrotated paper dimensions. NARROWLY is landscape.
        width, height = page['width'], page['height']
        if page['orientation'] == 'landscape' and width > height:
            width, height = height, width
        page_pr = element(section, HP, 'pagePr', landscape=landscape, width=width * HWP_PER_TWIP, height=height * HWP_PER_TWIP, gutterType='LEFT_ONLY')
        element(page_pr, HP, 'margin', **{key: page[key] * HWP_PER_TWIP for key in ['left', 'right', 'top', 'bottom', 'header', 'footer', 'gutter']})
        element(element(run, HP, 'ctrl'), HP, 'colPr', id='', type='NEWSPAPER', layout='LEFT', colCount=1, sameSz=1, sameGap=0)
        return run

    def _header(self):
        root = ET.Element('{' + HH + '}head', dict(version='1.5', secCnt=str(len(self.document.sections))))
        element(root, HH, 'beginNum', page=1, footnote=1, endnote=1, pic=1, tbl=1, equation=1)
        refs = element(root, HH, 'refList')
        fontfaces = element(refs, HH, 'fontfaces', itemCnt=7)
        for language in LANGUAGES:
            fontface = element(fontfaces, HH, 'fontface', lang=language, fontCnt=len(self.fonts))
            for index, font in enumerate(self.fonts):
                element(fontface, HH, 'font', id=index, face=font, type='TTF', isEmbedded=0)
        fills = element(refs, HH, 'borderFills', itemCnt=len(self.fills))
        for index, (borders, shading) in enumerate(self.fills, 1):
            fill = element(fills, HH, 'borderFill', id=index, threeD=0, shadow=0, centerLine='NONE')
            for side in ['left', 'right', 'top', 'bottom']:
                border = borders.get(side, {})
                element(fill, HH, side + 'Border', type=BORDER_TYPES.get(border.get('kind'), 'NONE'),
                        width=f"{border.get('size', 4) * MM_PER_BORDER_UNIT:.4f} mm", color='#' + border.get('colour', '000000'))
            if shading:
                element(element(fill, HC, 'fillBrush'), HC, 'winBrush', faceColor='#' + shading, hatchColor='#000000', alpha=0)
        characters = element(refs, HH, 'charProperties', itemCnt=len(self.char_styles))
        for index, style in enumerate(self.char_styles):
            char = element(characters, HH, 'charPr', id=index, height=round(style.get('size', 11) * 100),
                           textColor='#' + style.get('colour', '000000'), shadeColor='none', useFontSpace=0, useKerning=0)
            font_id = self.fonts.index(style.get('font', 'Arial'))
            for tag, val in [('fontRef', font_id), ('ratio', 100), ('spacing', 0), ('relSz', 100), ('offset', 0)]:
                element(char, HH, tag, **{key: val for key in FONT_KEYS})
            if style.get('bold'):
                element(char, HH, 'bold')
            if style.get('italic'):
                element(char, HH, 'italic')
            underline = style.get('underline', 'none')
            element(char, HH, 'underline', type='NONE' if underline == 'none' else 'BOTTOM',
                    shape=UNDERLINE_TYPES.get(underline, 'SOLID'), color='#' + style.get('colour', '000000'))
            element(char, HH, 'strikeout', shape='SOLID' if style.get('strike') else 'NONE', color='#' + style.get('colour', '000000'))
        element(element(refs, HH, 'tabProperties', itemCnt=1), HH, 'tabPr', id=0, autoTabLeft=0, autoTabRight=0)
        self._numbering(refs)
        properties = element(refs, HH, 'paraProperties', itemCnt=len(self.para_styles))
        for index, style in enumerate(self.para_styles):
            para = element(properties, HH, 'paraPr', id=index, tabPrIDRef=0, condense=0, fontLineHeight=0, snapToGrid=0)
            alignment = {'both': 'JUSTIFY', 'justify': 'JUSTIFY', 'start': 'LEFT', 'end': 'RIGHT'}.get(style.get('alignment'), style.get('alignment', 'left').upper())
            element(para, HH, 'align', horizontal=alignment, vertical='BASELINE')
            number = style.get('numbering')
            element(para, HH, 'heading', type=('BULLET' if number['kind'] == 'bullet' else 'NUMBER') if number else 'NONE',
                    idRef=self._number_id(number) if number else 0, level=number['level'] if number else 0)
            element(para, HH, 'breakSetting', breakLatinWord='KEEP_WORD', breakNonLatinWord='KEEP_WORD',
                    widowOrphan=int(style.get('widowControl', False)), keepWithNext=int(style.get('keepNext', False)),
                    keepLines=int(style.get('keepLines', False)), pageBreakBefore=0, lineWrap='BREAK')
            margin = element(para, HH, 'margin')
            for tag, value in [('intent', style.get('first', -style.get('hanging', 0))), ('left', style.get('left', 0)),
                               ('right', style.get('right', 0)), ('prev', style.get('before', 0)), ('next', style.get('after', 0))]:
                element(margin, HC, tag, value=value * HWP_PER_TWIP, unit='HWPUNIT')
            rule = style.get('lineRule', 'auto')
            line = style.get('line', 240)
            spacing_type = {'auto': 'PERCENT', 'exact': 'FIXED', 'atLeast': 'AT_LEAST'}.get(rule, 'PERCENT')
            element(para, HH, 'lineSpacing', type=spacing_type, value=round(line * 100 / 240) if rule == 'auto' else line * HWP_PER_TWIP, unit='HWPUNIT')
        styles = element(refs, HH, 'styles', itemCnt=10)
        for index in range(10):
            name = 'Normal' if index == 0 else f'Heading {index}'
            element(styles, HH, 'style', id=index, type='PARA', name=name, engName=name, paraPrIDRef=0,
                    charPrIDRef=0, nextStyleIDRef=0, langID=1042, lockForm=0)
        return root

    def _number_id(self, number):
        same_kind = [item for item in self.numbers if (item['kind'] == 'bullet') == (number['kind'] == 'bullet')]
        return same_kind.index(number) + 1

    def _numbering(self, refs):
        numberings = element(refs, HH, 'numberings', itemCnt=sum(number['kind'] != 'bullet' for number in self.numbers))
        bullets = element(refs, HH, 'bullets', itemCnt=sum(number['kind'] == 'bullet' for number in self.numbers))
        for number in self.numbers:
            index = self._number_id(number)
            attrs = dict(level=number['level'] + 1, align='LEFT', useInstWidth=1, autoIndent=1, widthAdjust=0,
                         textOffsetType='PERCENT', textOffset=50, charPrIDRef=0)
            if number['kind'] == 'bullet':
                bullet = element(bullets, HH, 'bullet', id=index, char=number.get('text', '•')[:1], useImage=0)
                element(bullet, HH, 'paraHead', **attrs)
            else:
                numbering = element(numberings, HH, 'numbering', id=index, start=number['start'])
                head = element(numbering, HH, 'paraHead', start=number['start'], numFormat='DIGIT', **attrs)
                head.text = number.get('text', '%1.').replace('%', '^')

    def write(self):
        manifest = ET.Element('{' + OPF + '}package', {'version': '3.0'})
        items = element(manifest, OPF, 'manifest')
        spine = element(manifest, OPF, 'spine')
        element(items, OPF, 'item', id='header', href='Contents/header.xml', **{'media-type': 'application/xml'})
        for index, section in enumerate(self.document.sections):
            root = ET.Element('{' + HS + '}sec')
            prefix = self._section_prefix(section.page)
            for block_index, block in enumerate(section.blocks or [Paragraph()]):
                first_prefix = prefix if block_index == 0 else None
                root.append(self._paragraph(block, first_prefix) if isinstance(block, Paragraph) else self._table(block, first_prefix))
            name = f'Contents/section{index}.xml'
            self.parts[name] = encoded(root)
            element(items, OPF, 'item', id=f'section{index}', href=name, **{'media-type': 'application/xml'})
            element(spine, OPF, 'itemref', idref=f'section{index}')
        for name in self.parts:
            if name.startswith('BinData/'):
                binary_id = name.rsplit('/', 1)[-1].split('.')[0]
                extension = name.rsplit('.', 1)[-1]
                media_type = {'png': 'image/png', 'jpg': 'image/jpeg', 'gif': 'image/gif', 'bmp': 'image/bmp'}[extension]
                element(items, OPF, 'item', id=binary_id, href=name, **{'media-type': media_type, 'isEmbeded': '1'})
        self.parts['Contents/header.xml'] = encoded(self._header())
        self.parts['Contents/content.hpf'] = encoded(manifest)
        self.parts['version.xml'] = b'<HCFVersion major="5" minor="0" micro="0" buildNumber="0"/>'
        self.parts['META-INF/container.xml'] = b'<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="Contents/content.hpf" media-type="application/hwp+zip"/></rootfiles></container>'
        return write_package(self.parts)


def write_hwpx(document):
    return Writer(document).write()
