"""Emit a direct OOXML package from the supported conversion model."""
from xml.etree import ElementTree as ET

from .docx_read import W, R, A, WP, REL, CT
from .model import Paragraph
from .package import element, encoded, write_package
from .style_index import indexed_value

PIC = 'http://schemas.openxmlformats.org/drawingml/2006/picture'
XML = 'http://www.w3.org/XML/1998/namespace'
for prefix, namespace in [('w', W), ('r', R), ('a', A), ('wp', WP), ('pic', PIC)]:
    ET.register_namespace(prefix, namespace)


def package_part(root, namespace):
    """Serialise <Types> or <Relationships> with an unprefixed namespace.

    LibreOffice matches these element names literally and cannot load a
    package whose parts say `ns0:Types`. ElementTree keeps one default namespace at a time, so
    set it immediately before each part is written.
    """
    ET.register_namespace('', namespace)
    return encoded(root)


def word(parent, tag, **attrs):
    node = element(parent, W, tag)
    node.attrib.update({'{' + W + '}' + key: str(value) for key, value in attrs.items()})
    return node


class Writer:
    def __init__(self, document):
        self.document = document
        self.parts = {}
        self.rels = ET.Element('{' + REL + '}Relationships')
        self.types = ET.Element('{' + CT + '}Types')
        self.numbers = []
        self.number_ids = {}
        self.image_count = 0

    def _run(self, parent, run):
        native = word(parent, 'r')
        properties = word(native, 'rPr')
        style = run.style
        if 'font' in style:
            word(properties, 'rFonts', ascii=style['font'], hAnsi=style['font'], eastAsia=style['font'])
        for key, tag in [('bold', 'b'), ('italic', 'i'), ('strike', 'strike')]:
            if key in style:
                word(properties, tag, val=int(style[key]))
        if 'size' in style:
            word(properties, 'sz', val=round(style['size'] * 2))
        if 'colour' in style:
            word(properties, 'color', val=style['colour'])
        if 'underline' in style:
            word(properties, 'u', val=style['underline'])
        if run.image:
            self._image(native, run.image)
        text = run.text.replace('\r\n', '\n').replace('\r', '\n')
        chunks = text.split('\n')
        for line_index, chunk in enumerate(chunks):
            if line_index:
                word(native, 'br')
            for tab_index, part in enumerate(chunk.split('\t')):
                if tab_index:
                    word(native, 'tab')
                if part or (not text and not run.image):
                    node = word(native, 't')
                    node.set('{' + XML + '}space', 'preserve')
                    node.text = part

    def _paragraph(self, parent, paragraph):
        node = word(parent, 'p')
        properties = word(node, 'pPr')
        if paragraph.heading:
            word(properties, 'pStyle', val=f'Heading{paragraph.heading}')
        style = paragraph.style
        if 'alignment' in style:
            word(properties, 'jc', val=style['alignment'])
        indents = {native: style[key] for key, native in [('left', 'left'), ('right', 'right'), ('first', 'firstLine'), ('hanging', 'hanging')] if key in style}
        if indents:
            word(properties, 'ind', **indents)
        spacing = {key: style[key] for key in ['before', 'after', 'line', 'lineRule'] if key in style}
        if spacing:
            word(properties, 'spacing', **spacing)
        for key in ['keepNext', 'keepLines', 'widowControl']:
            if key in style:
                word(properties, key, val=int(style[key]))
        if paragraph.page_break:
            word(properties, 'pageBreakBefore')
        if paragraph.numbering:
            number = dict(paragraph.numbering)
            number_id = indexed_value(self.numbers, self.number_ids, number) + 1
            numbering = word(properties, 'numPr')
            word(numbering, 'ilvl', val=number['level'])
            word(numbering, 'numId', val=number_id)
        for run in paragraph.runs:
            self._run(node, run)
        return node

    def _cell(self, parent, cell, *, continuation=False):
        tc = word(parent, 'tc')
        properties = word(tc, 'tcPr')
        if cell.colspan > 1:
            word(properties, 'gridSpan', val=cell.colspan)
        if continuation or cell.rowspan > 1:
            word(properties, 'vMerge', **({} if continuation else {'val': 'restart'}))
        if cell.borders:
            borders = word(properties, 'tcBorders')
            for side, border in cell.borders.items():
                word(borders, side, val=border['kind'], sz=border['size'], color=border['colour'])
        if cell.shading:
            word(properties, 'shd', val='clear', fill=cell.shading)
        if continuation:
            word(tc, 'p')
        else:
            for block in cell.blocks:
                self._block(tc, block)
            if not cell.blocks or not isinstance(cell.blocks[-1], Paragraph):
                word(tc, 'p')  # OOXML requires a final paragraph after a nested table.

    def _table(self, parent, table):
        node = word(parent, 'tbl')
        properties = word(node, 'tblPr')
        word(properties, 'tblW', w=sum(table.widths), type='dxa')
        word(properties, 'tblLayout', type='fixed')
        grid = word(node, 'tblGrid')
        for width in table.widths:
            word(grid, 'gridCol', w=width)
        active = {}
        for row_index, row in enumerate(table.rows):
            native_row = word(node, 'tr')
            positions = {cell.column: (cell, False) for cell in row}
            for column, (cell, last_row) in list(active.items()):
                if row_index <= last_row:
                    positions[column] = (cell, True)
                else:
                    del active[column]
            columns = sorted(positions)
            if columns and columns[0] > 0:
                word(word(native_row, 'trPr'), 'gridBefore', val=columns[0])
            cursor = columns[0] if columns else 0
            for column in columns:
                while cursor < column:
                    # Gaps need explicit empty cells so later cell addresses do not shift.
                    from .model import Cell
                    self._cell(native_row, Cell(blocks=[Paragraph()], column=cursor))
                    cursor += 1
                cell, continuation = positions[column]
                self._cell(native_row, cell, continuation=continuation)
                if cell.rowspan > 1 and not continuation:
                    active[column] = (cell, row_index + cell.rowspan - 1)
                cursor = column + cell.colspan
            if cursor < len(table.widths):
                row_pr = native_row.find('{' + W + '}trPr')
                if row_pr is None:
                    row_pr = ET.Element('{' + W + '}trPr')
                    native_row.insert(0, row_pr)
                word(row_pr, 'gridAfter', val=len(table.widths) - cursor)
        return node

    def _block(self, parent, block):
        return self._paragraph(parent, block) if isinstance(block, Paragraph) else self._table(parent, block)

    def _image(self, parent, image):
        self.image_count += 1
        index = self.image_count
        extension = {'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/bmp': 'bmp'}[image.media_type]
        name = f'word/media/image{index}.{extension}'
        self.parts[name] = image.data
        element(self.rels, REL, 'Relationship', Id=f'image{index}', Type=R + '/image', Target=f'media/image{index}.{extension}')
        element(self.types, CT, 'Override', PartName='/' + name, ContentType=image.media_type)
        drawing = word(parent, 'drawing')
        inline = element(drawing, WP, 'inline', distT=0, distB=0, distL=0, distR=0)
        width, height = image.width * 635, image.height * 635
        element(inline, WP, 'extent', cx=width, cy=height)
        element(inline, WP, 'docPr', id=index, name=image.name)
        graphic = element(inline, A, 'graphic')
        graphic_data = element(graphic, A, 'graphicData', uri=PIC)
        picture = element(graphic_data, PIC, 'pic')
        nonvisual = element(picture, PIC, 'nvPicPr')
        element(nonvisual, PIC, 'cNvPr', id=index, name=image.name)
        element(nonvisual, PIC, 'cNvPicPr')
        fill = element(picture, PIC, 'blipFill')
        blip = element(fill, A, 'blip')
        blip.set('{' + R + '}embed', f'image{index}')
        element(element(fill, A, 'stretch'), A, 'fillRect')
        shape = element(picture, PIC, 'spPr')
        transform = element(shape, A, 'xfrm')
        element(transform, A, 'off', x=0, y=0)
        element(transform, A, 'ext', cx=width, cy=height)
        element(element(shape, A, 'prstGeom', prst='rect'), A, 'avLst')

    def _page(self, node, page):
        section = word(node, 'sectPr')
        word(section, 'pgSz', w=page['width'], h=page['height'], orient=page['orientation'])
        word(section, 'pgMar', **{key: page[key] for key in ['left', 'right', 'top', 'bottom', 'header', 'footer', 'gutter']})
        return section

    def _styles(self):
        root = ET.Element('{' + W + '}styles')
        style = word(root, 'style', type='paragraph', styleId='Normal', default=1)
        word(style, 'name', val='Normal')
        for index in range(1, 10):
            style = word(root, 'style', type='paragraph', styleId=f'Heading{index}')
            word(style, 'name', val=f'heading {index}')
            word(style, 'basedOn', val='Normal')
            word(word(style, 'pPr'), 'outlineLvl', val=index - 1)
        return root

    def _numbering(self):
        root = ET.Element('{' + W + '}numbering')
        for index, number in enumerate(self.numbers, 1):
            abstract = word(root, 'abstractNum', abstractNumId=index)
            level = word(abstract, 'lvl', ilvl=number['level'])
            word(level, 'start', val=number['start'])
            word(level, 'numFmt', val=number['kind'])
            word(level, 'lvlText', val=number['text'])
            word(word(root, 'num', numId=index), 'abstractNumId', val=index)
        return root

    def write(self):
        root = ET.Element('{' + W + '}document')
        body = word(root, 'body')
        for index, section in enumerate(self.document.sections):
            for block in section.blocks:
                self._block(body, block)
            if index < len(self.document.sections) - 1:
                last = body[-1] if len(body) else None
                if last is None or last.tag != '{' + W + '}p':
                    last = word(body, 'p')
                properties = last.find('{' + W + '}pPr')
                if properties is None:
                    properties = ET.Element('{' + W + '}pPr')
                    last.insert(0, properties)
                self._page(properties, section.page)
            else:
                self._page(body, section.page)
        self.parts['word/document.xml'] = encoded(root)
        self.parts['word/styles.xml'] = encoded(self._styles())
        self.parts['word/numbering.xml'] = encoded(self._numbering())
        for name, kind in [('styles', 'styles'), ('numbering', 'numbering')]:
            element(self.rels, REL, 'Relationship', Id=name, Type=R + '/' + kind, Target=name + '.xml')
        self.parts['word/_rels/document.xml.rels'] = package_part(self.rels, REL)
        package_rels = ET.Element('{' + REL + '}Relationships')
        element(package_rels, REL, 'Relationship', Id='document', Type=R + '/officeDocument', Target='word/document.xml')
        self.parts['_rels/.rels'] = package_part(package_rels, REL)
        element(self.types, CT, 'Default', Extension='xml', ContentType='application/xml')
        element(self.types, CT, 'Default', Extension='rels', ContentType='application/vnd.openxmlformats-package.relationships+xml')
        for name, kind in [('document', 'document.main'), ('styles', 'styles'), ('numbering', 'numbering')]:
            element(self.types, CT, 'Override', PartName=f'/word/{name}.xml', ContentType=f'application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml')
        self.parts['[Content_Types].xml'] = package_part(self.types, CT)
        return write_package(self.parts)


def write_docx(document):
    return Writer(document).write()
