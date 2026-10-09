"""Read rich HWPX references beyond the preservation engine's text nodes."""
import re
from xml.etree import ElementTree as ET

from ..document import Document, HP, HC, HH, local
from ..errors import MAX_NODES, require
from .model import Cell, Image, Paragraph, RichDocument, Run, Section, Table, DEFAULT_PAGE
from .package import MAX_COLUMNS, MAX_GRID_WORK, integer, read_package
from .hwpx_write import HWP_PER_TWIP, MM_PER_BORDER_UNIT, BORDER_TYPES, UNDERLINE_TYPES, OPF


def find(node, namespace, tag):
    return node.find('{' + namespace + '}' + tag) if node is not None else None


def colour(value):
    return value.lstrip('#').upper() if value and re.fullmatch(r'#?[0-9a-fA-F]{6}', value) else None


def twips(value, default=0):
    return round(integer(value, default * HWP_PER_TWIP) / HWP_PER_TWIP)


class Reader:
    def __init__(self, native, trees):
        self.native, self.trees = native, trees
        self.document = RichDocument()
        self.count = 0
        self.chars, self.paras, self.styles, self.fills = {}, {}, {}, {}
        self.numbers, self.bullets, self.media = {}, {}, {}
        self._references()
        features = {'fieldBegin': 'fields', 'equation': 'equations', 'footNote': 'footnotes', 'endNote': 'endnotes',
                    'header': 'headers', 'footer': 'footers', 'rect': 'shapes', 'ellipse': 'shapes', 'line': 'shapes',
                    'polygon': 'shapes', 'curve': 'shapes', 'container': 'shapes', 'drawText': 'textBoxes',
                    'textBox': 'textBoxes', 'memo': 'comments', 'chart': 'charts', 'ole': 'embeddedObjects',
                    'trackChange': 'trackedChanges', 'trackChangeBegin': 'trackedChanges',
                    'bookmark': 'bookmarks', 'pageNum': 'pageNumbering', 'video': 'video', 'compose': 'composedText'}
        for name, tree in trees.items():
            for item in tree.iter():
                if item.tag.startswith('{' + HP + '}') and local(item.tag) in features:
                    self.document.loss(features[local(item.tag)])
            if name.startswith('Contents/masterpage'):
                self.document.loss('masterPages')
        for name in native.parts:
            if any(word in name.lower() for word in ('signature', 'scripts/', 'vbascript')):
                self.document.loss('activeOrSignedContent')
            if name.startswith('BinData/') and not any(item['name'] == name for item in self.media.values()):
                self.document.loss('unreferencedBinaryParts')

    def _references(self):
        header = self.trees['Contents/header.xml']
        fonts = {}
        for face in header.iter('{' + HH + '}fontface'):
            if face.get('lang') in {'LATIN', 'HANGUL'}:
                for font in face:
                    fonts[font.get('id')] = font.get('face', 'Arial')
        for char in header.iter('{' + HH + '}charPr'):
            style = dict(size=integer(char.get('height'), 1100, minimum=1) / 100,
                         colour=colour(char.get('textColor')) or '000000')
            font = find(char, HH, 'fontRef')
            style['font'] = fonts.get(font.get('latin') or font.get('hangul'), 'Arial') if font is not None else 'Arial'
            style['bold'] = find(char, HH, 'bold') is not None
            style['italic'] = find(char, HH, 'italic') is not None
            underline = find(char, HH, 'underline')
            strike = find(char, HH, 'strikeout')
            reverse_underlines = {value: key for key, value in UNDERLINE_TYPES.items()}
            style['underline'] = reverse_underlines.get(underline.get('shape'), 'single') if underline is not None and underline.get('type') != 'NONE' else 'none'
            style['strike'] = strike is not None and strike.get('shape') not in {'NONE', None}
            self.chars[char.get('id')] = style
            for item in char:
                if local(item.tag) not in {'fontRef', 'ratio', 'spacing', 'relSz', 'offset', 'bold', 'italic', 'underline', 'strikeout', 'outline', 'shadow'}:
                    self.document.loss('characterProperty:' + local(item.tag))
                elif local(item.tag) in {'ratio', 'spacing', 'relSz', 'offset'}:
                    expected = '100' if local(item.tag) in {'ratio', 'relSz'} else '0'
                    if any(value != expected for value in item.attrib.values()):
                        self.document.loss('characterProperty:' + local(item.tag))
                elif local(item.tag) in {'outline', 'shadow'} and item.get('type', 'NONE') != 'NONE':
                    self.document.loss('characterProperty:' + local(item.tag))
        for para in header.iter('{' + HH + '}paraPr'):
            self.paras[para.get('id')] = para
        for style in header.iter('{' + HH + '}style'):
            name = style.get('engName', '') or style.get('name', '')
            heading = re.fullmatch(r'heading\s*([1-9])', name, re.I)
            self.styles[style.get('id')] = int(heading[1]) if heading else 0
        reverse_borders = {value: key for key, value in BORDER_TYPES.items()}
        for fill in header.iter('{' + HH + '}borderFill'):
            borders = {}
            for side in ['left', 'right', 'top', 'bottom']:
                border = find(fill, HH, side + 'Border')
                if border is None or border.get('type') == 'NONE':
                    continue
                width = border.get('width', '0.1 mm').removesuffix(' mm')
                require(bool(re.fullmatch(r'[0-9]+(?:\.[0-9]+)?', width)) and len(width) <= 20,
                        'BAD_XML', 'Invalid HWPX border width.')
                borders[side] = dict(kind=reverse_borders.get(border.get('type'), 'single'),
                                     size=round(float(width) / MM_PER_BORDER_UNIT), colour=colour(border.get('color')) or '000000')
                if border.get('type') not in reverse_borders:
                    self.document.loss('borderStyles')
            brush = fill.find('.//{' + HC + '}winBrush')
            self.fills[fill.get('id')] = (borders, colour(brush.get('faceColor')) if brush is not None else None)
            if any(local(item.tag) in {'gradBrush', 'imgBrush'} for item in fill.iter()):
                self.document.loss('complexCellShading')
        for numbering in header.iter('{' + HH + '}numbering'):
            levels = {}
            for head in numbering:
                level = integer(head.get('level'), 1, minimum=1, maximum=10) - 1
                if head.get('numFormat') != 'DIGIT':
                    continue
                levels[level] = dict(kind='decimal', level=min(level, 8), start=integer(head.get('start'), integer(numbering.get('start'), 1)),
                                     text=(head.text or '^1.').replace('^', '%'), id=numbering.get('id'))
            self.numbers[numbering.get('id')] = levels
        for bullet in header.iter('{' + HH + '}bullet'):
            head = find(bullet, HH, 'paraHead')
            self.bullets[bullet.get('id')] = dict(kind='bullet', level=max(0, integer(head.get('level') if head is not None else None, 1, minimum=0, maximum=9) - 1),
                                                start=1, text=bullet.get('char', '•'), id=bullet.get('id'))
        manifest = self.trees['Contents/content.hpf']
        for item in manifest.iter('{' + OPF + '}item'):
            name = item.get('href', '')
            if name in self.native.parts and (name.startswith('BinData/') or item.get('media-type', '').startswith('image/')):
                self.media[item.get('id')] = dict(name=name, media_type=item.get('media-type', ''))

    def _para_style(self, node):
        result, number = {}, None
        if node is None:
            return result, number
        alignment = find(node, HH, 'align')
        if alignment is not None:
            result['alignment'] = {'JUSTIFY': 'both'}.get(alignment.get('horizontal'), alignment.get('horizontal', 'LEFT').lower())
        # Native header margins can occur inside a namespace switch. The default
        # branch supplies HWPUNIT values and is preferred over character units.
        default = node.find('.//{' + HP + '}default')
        scope = default if default is not None else node
        margin = scope.find('.//{' + HH + '}margin')
        if margin is not None:
            for item in margin:
                key = {'intent': 'first', 'left': 'left', 'right': 'right', 'prev': 'before', 'next': 'after'}.get(local(item.tag))
                if key:
                    result[key] = twips(item.get('value'))
            if result.get('first', 0) < 0:
                result['hanging'] = -result.pop('first')
        spacing = scope.find('.//{' + HH + '}lineSpacing')
        if spacing is not None:
            rule = {'PERCENT': 'auto', 'FIXED': 'exact', 'AT_LEAST': 'atLeast'}.get(spacing.get('type'), 'auto')
            result['lineRule'] = rule
            result['line'] = round(integer(spacing.get('value'), 100) * 240 / 100) if rule == 'auto' else twips(spacing.get('value'))
        breaking = find(node, HH, 'breakSetting')
        if breaking is not None:
            for source, target in [('keepWithNext', 'keepNext'), ('keepLines', 'keepLines'), ('widowOrphan', 'widowControl'), ('pageBreakBefore', 'pageBreakBefore')]:
                if breaking.get(source) == '1':
                    result[target] = True
        heading = find(node, HH, 'heading')
        if heading is not None and heading.get('type') in {'NUMBER', 'BULLET'}:
            if heading.get('type') == 'BULLET':
                number = self.bullets.get(heading.get('idRef'))
            else:
                number = self.numbers.get(heading.get('idRef'), {}).get(integer(heading.get('level'), 0, minimum=0, maximum=9))
            if not number:
                self.document.loss('complexNumbering')
        return result, dict(number) if number else None

    def _image(self, picture):
        image = picture.find('.//{' + HC + '}img')
        reference = image.get('binaryItemIDRef') if image is not None else None
        item = self.media.get(reference)
        if item is None:
            self.document.loss('unavailableImages')
            return None
        media_type = item['media_type']
        if media_type not in {'image/png', 'image/jpeg', 'image/gif', 'image/bmp'}:
            self.document.loss('unsupportedImageTypes')
            return None
        size = find(picture, HP, 'sz')
        if size is None:
            size = find(picture, HP, 'curSz')
        width = twips(size.get('width'), 1440) if size is not None else 1440
        height = twips(size.get('height'), 1440) if size is not None else 1440
        position = find(picture, HP, 'pos')
        if position is not None and position.get('treatAsChar') != '1':
            self.document.loss('floatingImagePlacement')
        rotation = find(picture, HP, 'rotationInfo')
        if rotation is not None and rotation.get('angle', '0') != '0':
            self.document.loss('imageRotation')
        flip = find(picture, HP, 'flip')
        if flip is not None and any(flip.get(key, '0') not in {'0', 'false'} for key in ['horizontal', 'vertical']):
            self.document.loss('imageTransforms')
        original_size = find(picture, HP, 'orgSz')
        clip = find(picture, HP, 'imgClip')
        if clip is not None and original_size is not None:
            bounds = {key: integer(clip.get(key), 0) for key in ['left', 'right', 'top', 'bottom']}
            full = dict(left=0, top=0, right=integer(original_size.get('width')), bottom=integer(original_size.get('height')))
            if any(bounds.values()) and bounds != full:
                self.document.loss('imageCropping')
        if image is not None and (any(image.get(key, '0') != '0' for key in ['bright', 'contrast', 'alpha'])
                                  or image.get('effect', 'REAL_PIC') != 'REAL_PIC'):
            self.document.loss('imageEffects')
        return Image(self.native.parts[item['name']], media_type, max(1, width), max(1, height), item['name'].rsplit('/', 1)[-1])

    def _text(self, node):
        text = node.text or ''
        for item in node:
            tag = local(item.tag)
            controls = {'lineBreak': '\n', 'tab': '\t', 'nbSpace': '\u00a0', 'fwSpace': '\u2007', 'hyphen': '\u00ad'}
            if tag in controls:
                text += controls[tag]
            elif tag == 'columnBreak':
                text += '\n'
                self.document.loss('columnBreaks')
            else:
                self.document.loss('textControl:' + tag)
                text += self._text(item)
            text += item.tail or ''
        return text

    def _paragraph(self, node):
        self.count += 1
        require(self.count <= MAX_NODES, 'LIMIT', 'Too many conversion paragraphs.')
        style, number = self._para_style(self.paras.get(node.get('paraPrIDRef')))
        paragraph = Paragraph(style=style, heading=self.styles.get(node.get('styleIDRef'), 0), numbering=number,
                              page_break=node.get('pageBreak') == '1' or style.pop('pageBreakBefore', False))
        blocks = []
        has_own_content = False
        has_table = any(item.tag == '{' + HP + '}tbl' for run in node.findall('{' + HP + '}run') for item in run)
        for run in node.findall('{' + HP + '}run'):
            char = dict(self.chars.get(run.get('charPrIDRef'), {}))
            for item in run:
                tag = local(item.tag)
                if tag == 't':
                    text = self._text(item)
                    if has_table and not text:
                        continue
                    paragraph.runs.append(Run(text, dict(char)))
                    has_own_content = True
                elif tag == 'pic':
                    image = self._image(item)
                    if image:
                        paragraph.runs.append(Run(style=dict(char), image=image))
                        has_own_content = True
                elif tag == 'tbl':
                    if has_own_content:
                        blocks.append(paragraph)
                        paragraph = Paragraph(style=dict(style))
                        has_own_content = False
                    blocks.append(self._table(item))
                elif tag in {'secPr', 'ctrl'}:
                    for control in item:
                        if local(control.tag) not in {'colPr', 'pagePr', 'margin', 'grid', 'startNum', 'visibility', 'lineNumberShape', 'footNotePr', 'endNotePr', 'pageBorderFill'}:
                            # Known unsupported controls were already counted globally.
                            if local(control.tag) not in {'header', 'footer', 'footNote', 'endNote', 'fieldBegin', 'fieldEnd'}:
                                self.document.loss('control:' + local(control.tag))
                        if local(control.tag) == 'colPr' and control.get('colCount', '1') != '1':
                            self.document.loss('multipleColumns')
                elif tag not in {'equation', 'ole', 'chart', 'rect', 'ellipse', 'line', 'polygon', 'curve', 'container', 'fieldBegin', 'fieldEnd', 'footNote', 'endNote', 'header', 'footer'}:
                    self.document.loss('runElement:' + tag)
        if has_own_content or not blocks:
            if not paragraph.runs:
                paragraph.runs.append(Run())
            blocks.append(paragraph)
        return blocks

    def _table(self, node):
        count = integer(node.get('colCnt'), 1, minimum=1, maximum=MAX_COLUMNS)
        row_count = integer(node.get('rowCnt'), 1, minimum=1, maximum=MAX_NODES)
        require(count * row_count <= MAX_GRID_WORK, 'LIMIT', 'Table grid exceeds work limit.')
        table = Table(rows=[[] for _ in range(row_count)], widths=[0] * count)
        occupancy = set()
        constraints = []
        for row_index, row in enumerate(node.findall('{' + HP + '}tr')):
            column = 0
            for tc in row.findall('{' + HP + '}tc'):
                address = find(tc, HP, 'cellAddr')
                span = find(tc, HP, 'cellSpan')
                row_at = integer(address.get('rowAddr'), row_index, minimum=0, maximum=row_count - 1) if address is not None else row_index
                col_at = integer(address.get('colAddr'), column, minimum=0, maximum=count - 1) if address is not None else column
                colspan = integer(span.get('colSpan'), 1, minimum=1, maximum=count) if span is not None else 1
                rowspan = integer(span.get('rowSpan'), 1, minimum=1, maximum=row_count) if span is not None else 1
                require(col_at + colspan <= count and row_at + rowspan <= row_count, 'BAD_XML', 'Table span leaves grid.')
                for r in range(row_at, row_at + rowspan):
                    for c in range(col_at, col_at + colspan):
                        require((r, c) not in occupancy, 'BAD_XML', 'Overlapping native table cells.')
                        occupancy.add((r, c))
                borders, shading = self.fills.get(tc.get('borderFillIDRef'), ({}, None))
                cell = Cell(column=col_at, colspan=colspan, rowspan=rowspan, borders=dict(borders), shading=shading)
                sublist = find(tc, HP, 'subList')
                if sublist is not None:
                    for paragraph in sublist.findall('{' + HP + '}p'):
                        cell.blocks.extend(self._paragraph(paragraph))
                table.rows[row_at].append(cell)
                size = find(tc, HP, 'cellSz')
                if size is not None:
                    width = twips(size.get('width'), 1440)
                    constraints.append((col_at, colspan, width))
                    if colspan == 1:
                        table.widths[col_at] = width
                column = col_at + colspan
        for column, span, width in constraints:
            unknown = [i for i in range(column, column + span) if table.widths[i] == 0]
            if unknown:
                remaining = width - sum(table.widths[column:column + span])
                for i in unknown:
                    table.widths[i] = max(1, round(remaining / len(unknown)))
        table.widths = [width or 1440 for width in table.widths]
        for row in table.rows:
            row.sort(key=lambda cell: cell.column)
        return table

    def _page(self, root):
        page = dict(DEFAULT_PAGE)
        properties = root.find('.//{' + HP + '}pagePr')
        if properties is None:
            return page
        page['width'] = twips(properties.get('width'), page['width'])
        page['height'] = twips(properties.get('height'), page['height'])
        page['orientation'] = 'landscape' if properties.get('landscape') == 'NARROWLY' else 'portrait'
        if page['orientation'] == 'landscape' and page['width'] < page['height']:
            page['width'], page['height'] = page['height'], page['width']
        margin = find(properties, HP, 'margin')
        if margin is not None:
            for key in ['left', 'right', 'top', 'bottom', 'header', 'footer', 'gutter']:
                page[key] = twips(margin.get(key), page[key])
        return page

    def read(self):
        names = sorted((name for name in self.trees if re.fullmatch(r'Contents/section\d+\.xml', name)),
                       key=lambda name: integer(re.search(r'(\d+)\.xml$', name)[1]))
        for name in names:
            root = self.trees[name]
            section = Section(page=self._page(root))
            for item in root:
                if item.tag == '{' + HP + '}p':
                    section.blocks.extend(self._paragraph(item))
                else:
                    self.document.loss('sectionElement:' + local(item.tag))
            self.document.sections.append(section)
        return self.document


def read_hwpx(payload):
    _, trees = read_package(payload)
    native = Document(payload)
    require(native.format == 'hwpx', 'NOT_HWPX', 'Rich reader requires HWPX.')
    return Reader(native, trees).read()
