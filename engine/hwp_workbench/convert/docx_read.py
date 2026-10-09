"""Read supported OOXML content directly; never resolve external relationships."""
import re
from xml.etree import ElementTree as ET

from ..errors import MAX_NODES, require
from .model import Cell, Image, Paragraph, RichDocument, Run, Section, Table, DEFAULT_PAGE, paragraphs, split_page_breaks
from .package import MAX_COLUMNS, MAX_GRID_WORK, integer, member_target, read_package

W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
R = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
A = 'http://schemas.openxmlformats.org/drawingml/2006/main'
WP = 'http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing'
REL = 'http://schemas.openxmlformats.org/package/2006/relationships'
CT = 'http://schemas.openxmlformats.org/package/2006/content-types'


def w(tag):
    return '{' + W + '}' + tag


def attr(node, key='val', default=None):
    return node.get(w(key), default) if node is not None else default


def value(parent, tag, default=None):
    return attr(parent.find(w(tag)), default=default) if parent is not None else default


def toggle(node):
    return attr(node, default='1') not in {'0', 'false', 'off'}


def colour(value):
    return value.upper() if value and re.fullmatch('[0-9A-Fa-f]{6}', value) else None


class Reader:
    def __init__(self, parts, trees):
        self.parts, self.trees = parts, trees
        self.document = RichDocument()
        self.count = 0
        self.styles = {}
        self.style_properties = {}
        self.default_style = None
        self.default_run = {}
        self.default_para = {}
        self.numbers = {}
        self.rels = {}
        self.media_types = {}
        self.media_overrides = {}
        types = trees.get('[Content_Types].xml')
        require(types is not None and types.tag == '{' + CT + '}Types', 'NOT_DOCX', 'Missing DOCX content types.')
        for item in types:
            if item.tag == '{' + CT + '}Default':
                self.media_types[item.get('Extension', '').lower()] = item.get('ContentType', '')
            elif item.tag == '{' + CT + '}Override':
                self.media_overrides[item.get('PartName', '').lstrip('/')] = item.get('ContentType', '')
        for item in trees.get('word/_rels/document.xml.rels', ET.Element('empty')):
            if item.tag == '{' + REL + '}Relationship':
                self.rels[item.get('Id')] = item.attrib
                if item.get('TargetMode') == 'External':
                    self.document.loss('externalRelationships')
        self._read_styles()
        self._read_numbering()
        self._unsupported_parts()

    def _run_style(self, properties):
        result = {}
        if properties is None:
            return result
        supported = {'b', 'i', 'u', 'strike', 'sz', 'color', 'rFonts', 'rStyle'}
        for item in properties:
            tag = item.tag.rsplit('}', 1)[-1]
            if tag in {'b', 'i', 'strike'}:
                result[{'b': 'bold', 'i': 'italic', 'strike': 'strike'}[tag]] = toggle(item)
            elif tag == 'u':
                kind = attr(item, default='single')
                result['underline'] = kind
                if kind not in {'single', 'double', 'none', 'dotted', 'dash', 'wave'}:
                    self.document.loss('underlineStyles')
            elif tag == 'sz':
                result['size'] = integer(attr(item), 22, minimum=1, maximum=4000) / 2
            elif tag == 'color':
                result['colour'] = colour(attr(item)) or '000000'
                if item.get(w('themeColor')):
                    self.document.loss('themeColours')
            elif tag == 'rFonts':
                face = attr(item, 'eastAsia') or attr(item, 'ascii') or attr(item, 'hAnsi')
                if face:
                    result['font'] = face
                if any('Theme' in key for key in item.attrib):
                    self.document.loss('themeFonts')
            elif tag not in supported and tag != 'rPrChange':
                self.document.loss('runProperty:' + tag)
        return result

    def _para_style(self, properties):
        result = {}
        if properties is None:
            return result
        for item in properties:
            tag = item.tag.rsplit('}', 1)[-1]
            if tag == 'jc':
                result['alignment'] = attr(item, default='left')
            elif tag == 'ind':
                for source, target in [('left', 'left'), ('right', 'right'), ('firstLine', 'first'), ('hanging', 'hanging')]:
                    if attr(item, source) is not None:
                        result[target] = integer(attr(item, source))
                if any(key in item.attrib for key in (w('leftChars'), w('rightChars'), w('firstLineChars'), w('hangingChars'))):
                    self.document.loss('characterIndents')
            elif tag == 'spacing':
                for key in ['before', 'after', 'line']:
                    if attr(item, key) is not None:
                        result[key] = integer(attr(item, key))
                result['lineRule'] = attr(item, 'lineRule', 'auto')
            elif tag in {'pageBreakBefore', 'keepNext', 'keepLines', 'widowControl'}:
                result[tag] = toggle(item)
            elif tag not in {'pStyle', 'numPr', 'outlineLvl', 'sectPr', 'rPr', 'pPrChange'}:
                self.document.loss('paragraphProperty:' + tag)
        return result

    def _read_styles(self):
        root = self.trees.get('word/styles.xml')
        if root is None:
            return
        defaults = root.find(w('docDefaults'))
        if defaults is not None:
            self.default_run = self._run_style(defaults.find('./' + w('rPrDefault') + '/' + w('rPr')))
            self.default_para = self._para_style(defaults.find('./' + w('pPrDefault') + '/' + w('pPr')))
        for style in root.findall(w('style')):
            style_id = attr(style, 'styleId')
            self.styles[style_id] = style
            if attr(style, 'type', 'paragraph') == 'paragraph' and attr(style, 'default') in {'1', 'true', 'on'}:
                self.default_style = style_id
            self.style_properties[style_id] = (self._run_style(style.find(w('rPr'))), self._para_style(style.find(w('pPr'))))

    def _style(self, style_id, *, include_defaults=True):
        chain = []
        seen = set()
        while style_id in self.styles:
            require(style_id not in seen and len(chain) < 128, 'BAD_XML', 'Cyclic or excessive DOCX style inheritance.')
            seen.add(style_id)
            style = self.styles[style_id]
            chain.append(style)
            style_id = value(style, 'basedOn')
        run = dict(self.default_run) if include_defaults else {}
        para = dict(self.default_para) if include_defaults else {}
        heading = 0
        numbering = None
        for style in reversed(chain):
            properties = style.find(w('pPr'))
            inherited_run, inherited_para = self.style_properties[attr(style, 'styleId')]
            run.update(inherited_run)
            para.update(inherited_para)
            outline = value(properties, 'outlineLvl')
            if outline is not None:
                heading = min(9, integer(outline, minimum=0, maximum=9) + 1)
            elif re.fullmatch(r'heading\s*[1-9]', value(style, 'name', '').lower()):
                heading = int(value(style, 'name')[-1])
            if properties is not None and properties.find(w('numPr')) is not None:
                numbering = self._number(properties)
        return run, para, heading, numbering

    def _read_numbering(self):
        root = self.trees.get('word/numbering.xml')
        if root is None:
            return
        abstracts = {attr(item, 'abstractNumId'): item for item in root.findall(w('abstractNum'))}
        for number in root.findall(w('num')):
            abstract = abstracts.get(value(number, 'abstractNumId'))
            if abstract is None:
                continue
            levels = {}
            for level in abstract.findall(w('lvl')):
                index = integer(attr(level, 'ilvl'), minimum=0, maximum=8)
                kind = value(level, 'numFmt', 'decimal')
                text = value(level, 'lvlText', '•' if kind == 'bullet' else '%1.')
                levels[index] = dict(kind=kind, text=text, level=index, start=integer(value(level, 'start'), 1),
                                     id=attr(number, 'numId'), style=self._para_style(level.find(w('pPr'))))
            for override in number.findall(w('lvlOverride')):
                index = integer(attr(override, 'ilvl'), minimum=0, maximum=8)
                if index in levels and value(override, 'startOverride') is not None:
                    levels[index]['start'] = integer(value(override, 'startOverride'), 1)
                if override.find(w('lvl')) is not None:
                    self.document.loss('numberingOverrides')
            self.numbers[attr(number, 'numId')] = levels

    def _number(self, properties):
        number = properties.find(w('numPr')) if properties is not None else None
        if number is None:
            return None
        num_id = value(number, 'numId', '0')
        if num_id == '0':
            return None
        level = integer(value(number, 'ilvl'), 0, minimum=0, maximum=8)
        result = self.numbers.get(num_id, {}).get(level)
        if result is None:
            self.document.loss('unresolvedNumbering')
        elif result['kind'] not in {'bullet', 'decimal'}:
            self.document.loss('complexNumbering')
            return None
        return dict(result) if result else None

    def _unsupported_parts(self):
        features = {'header': 'headers', 'footer': 'footers', 'footnotes': 'footnotes',
                    'endnotes': 'endnotes', 'comments': 'comments'}
        for name, tree in self.trees.items():
            self.document.loss('textBoxes', len(list(tree.iter(w('txbxContent')))))
            for item in tree.iter():
                tag = item.tag.rsplit('}', 1)[-1]
                if item.tag.startswith('{' + W + '}') and tag in {'ins', 'del', 'moveFrom', 'moveTo', 'rPrChange', 'pPrChange', 'tblPrChange', 'tcPrChange', 'trPrChange', 'sectPrChange'}:
                    self.document.loss('trackedChanges')
                if item.tag == w('fldSimple') or (item.tag == w('fldChar') and attr(item, 'fldCharType') == 'begin'):
                    self.document.loss('fields')
            stem = name.rsplit('/', 1)[-1].split('.')[0]
            for prefix, feature in features.items():
                if stem.startswith(prefix):
                    if prefix in {'header', 'footer'}:
                        self.document.loss(feature)
                    else:
                        tag = {'footnotes': 'footnote', 'endnotes': 'endnote', 'comments': 'comment'}[prefix]
                        self.document.loss(feature, sum(1 for item in tree.findall(w(tag)) if integer(attr(item, 'id'), 0) >= 0))
            if '/charts/' in name:
                self.document.loss('charts')
        for name in self.parts:
            if 'vbaProject' in name or name.startswith('_xmlsignatures/'):
                self.document.loss('activeOrSignedContent')

    def _image(self, drawing):
        blip = drawing.find('.//{' + A + '}blip')
        if blip is None:
            self.document.loss('shapes')
            return None
        relation = self.rels.get(blip.get('{' + R + '}embed'))
        if not relation or relation.get('TargetMode') == 'External':
            self.document.loss('unavailableImages')
            return None
        name = member_target('word/document.xml', relation.get('Target'))
        data = self.parts.get(name)
        if data is None:
            self.document.loss('unavailableImages')
            return None
        media_type = self.media_overrides.get(name) or self.media_types.get(name.rsplit('.', 1)[-1].lower(), '')
        if media_type not in {'image/png', 'image/jpeg', 'image/gif', 'image/bmp'}:
            self.document.loss('unsupportedImageTypes')
            return None
        if drawing.find('{' + WP + '}anchor') is not None:
            self.document.loss('floatingImagePlacement')
        crop = drawing.find('.//{' + A + '}srcRect')
        if crop is not None and any(value != '0' for value in crop.attrib.values()):
            self.document.loss('imageCropping')
        transform = drawing.find('.//{' + A + '}xfrm')
        if transform is not None and any(transform.get(key, '0') not in {'0', 'false'} for key in ['rot', 'flipH', 'flipV']):
            self.document.loss('imageTransforms')
        if any(item.tag.rsplit('}', 1)[-1] in {'alphaModFix', 'duotone', 'grayscl', 'lum', 'biLevel'} for item in drawing.iter()):
            self.document.loss('imageEffects')
        extent = drawing.find('.//{' + WP + '}extent')
        width = integer(extent.get('cx'), 914400, minimum=1, maximum=1_000_000_000) if extent is not None else 914400
        height = integer(extent.get('cy'), 914400, minimum=1, maximum=1_000_000_000) if extent is not None else 914400
        # OOXML image extents use EMUs; 635 EMUs equal one twip.
        return Image(data, media_type, max(1, round(width / 635)), max(1, round(height / 635)), name.rsplit('/', 1)[-1])

    def _paragraph(self, node):
        self.count += 1
        require(self.count <= MAX_NODES, 'LIMIT', 'Too many conversion paragraphs.')
        properties = node.find(w('pPr'))
        run_style, para_style, heading, numbering = self._style(value(properties, 'pStyle') or self.default_style)
        para_style.update(self._para_style(properties))
        outline = value(properties, 'outlineLvl')
        if outline is not None:
            heading = min(9, integer(outline, minimum=0, maximum=9) + 1)
        if properties is not None and properties.find(w('numPr')) is not None:
            numbering = self._number(properties)
        if numbering:
            para_style = numbering.get('style', {}) | para_style
        paragraph = Paragraph(style=para_style, heading=heading, numbering=numbering,
                              page_break=para_style.pop('pageBreakBefore', False))

        def visit(parent):
            for child in parent:
                if child.tag == w('pPr'):
                    continue
                if child.tag == w('r'):
                    props = child.find(w('rPr'))
                    character_style = self._style(value(props, 'rStyle'), include_defaults=False)[0] if value(props, 'rStyle') else {}
                    style = run_style | character_style | self._run_style(props)
                    for item in child:
                        tag = item.tag
                        if tag in {w('t'), w('delText')}:
                            paragraph.runs.append(Run(item.text or '', dict(style)))
                        elif tag == w('br'):
                            if attr(item, 'type', 'textWrapping') == 'page':
                                paragraph.runs.append(Run('\f', dict(style)))
                                continue
                            elif attr(item, 'type', 'textWrapping') == 'column':
                                self.document.loss('columnBreaks')
                            paragraph.runs.append(Run('\n', dict(style)))
                        elif tag == w('cr'):
                            paragraph.runs.append(Run('\n', dict(style)))
                        elif tag == w('tab'):
                            paragraph.runs.append(Run('\t', dict(style)))
                        elif tag == w('drawing'):
                            image = self._image(item)
                            if image:
                                paragraph.runs.append(Run(style=dict(style), image=image))
                        elif tag in {w('fldChar'), w('instrText')}:
                            pass  # Count native field beginnings once, not each instruction fragment.
                        elif tag in {w('footnoteReference'), w('endnoteReference'), w('commentReference')}:
                            pass  # Count the referenced parts once, rather than each reference.
                        elif tag != w('rPr'):
                            feature = {'object': 'embeddedObjects', 'pict': 'shapes'}.get(tag.rsplit('}', 1)[-1], 'runElement:' + tag.rsplit('}', 1)[-1])
                            self.document.loss(feature)
                else:
                    feature = {'fldSimple': 'fields', 'hyperlink': 'hyperlinks', 'ins': 'trackedChanges',
                               'del': 'trackedChanges', 'moveFrom': 'trackedChanges', 'moveTo': 'trackedChanges',
                               'txbxContent': 'textBoxes'}.get(child.tag.rsplit('}', 1)[-1])
                    if child.tag.startswith('{http://schemas.openxmlformats.org/officeDocument/2006/math}'):
                        if child.tag.endswith('}oMath'):
                            self.document.loss('equations')
                        elif child.tag.endswith('}oMathPara'):
                            self.document.loss('equations', len(child.findall('{http://schemas.openxmlformats.org/officeDocument/2006/math}oMath')))
                        continue
                    if feature not in {'fields', 'trackedChanges', 'textBoxes'}:
                        self.document.loss(feature or 'paragraphElement:' + child.tag.rsplit('}', 1)[-1])
                    visit(child)
        visit(node)
        if not paragraph.runs:
            paragraph.runs.append(Run('', run_style))
        return paragraph

    def _table(self, node):
        grid = node.find(w('tblGrid'))
        widths = [integer(attr(item, 'w'), 1440, minimum=1) for item in grid] if grid is not None else []
        require(len(widths) <= MAX_COLUMNS, 'LIMIT', 'Too many table columns.')
        table = Table(widths=widths)
        active = {}
        table_properties = node.find(w('tblPr'))
        inherited_borders = self._borders(table_properties.find(w('tblBorders')) if table_properties is not None else None)
        work = 0
        for row_index, row in enumerate(node.findall(w('tr'))):
            require(row_index < MAX_NODES, 'LIMIT', 'Too many table rows.')
            row_props = row.find(w('trPr'))
            column = integer(value(row_props, 'gridBefore'), 0, minimum=0, maximum=MAX_COLUMNS)
            cells = []
            next_active = {}
            for cell_node in row.findall(w('tc')):
                properties = cell_node.find(w('tcPr'))
                span = integer(value(properties, 'gridSpan'), 1, minimum=1, maximum=MAX_COLUMNS)
                require(column + span <= MAX_COLUMNS, 'LIMIT', 'Table cell exceeds column limits.')
                merge = properties.find(w('vMerge')) if properties is not None else None
                if merge is not None and attr(merge, default='continue') != 'restart':
                    owner = active.get(column)
                    if owner is not None and owner.colspan == span:
                        owner.rowspan += 1
                        next_active[column] = owner
                        # Empty continuation cells are geometry, not source paragraphs.
                        continuation = self._blocks(cell_node)
                        has_content = any(
                            isinstance(block, Table) or block.page_break
                            or any(run.text or run.image is not None for run in block.runs)
                            for block in continuation
                        )
                        if has_content:
                            owner.blocks.extend(continuation)
                            self.document.loss('nonemptyMergeContinuation')
                        column += span
                        continue
                    self.document.loss('unresolvedTableMerge')
                cell = Cell(self._blocks(cell_node), column, span)
                if properties is not None:
                    cell.borders.update(self._borders(properties.find(w('tcBorders'))))
                    cell.shading = colour(attr(properties.find(w('shd')), 'fill'))
                    for prop in properties:
                        if prop.tag not in {w(tag) for tag in ('gridSpan', 'vMerge', 'tcBorders', 'shd', 'tcW')}:
                            self.document.loss('cellProperty:' + prop.tag.rsplit('}', 1)[-1])
                if merge is not None:
                    next_active[column] = cell
                cells.append(cell)
                column += span
                work += span
                require(work <= MAX_GRID_WORK, 'LIMIT', 'Table mapping exceeds work limit.')
            active = next_active
            table.rows.append(cells)
            while len(table.widths) < column:
                table.widths.append(1440)
        if not table.widths:
            table.widths = [1440]
        for row_index, row in enumerate(table.rows):
            for cell in row:
                inherited = {}
                for side, is_outer, inside in [
                    ('top', row_index == 0, 'insideH'),
                    ('bottom', row_index + cell.rowspan == len(table.rows), 'insideH'),
                    ('left', cell.column == 0, 'insideV'),
                    ('right', cell.column + cell.colspan == len(table.widths), 'insideV'),
                ]:
                    source = side if is_outer else inside
                    if source in inherited_borders:
                        inherited[side] = inherited_borders[source]
                cell.borders = inherited | cell.borders
        if table_properties is not None:
            for prop in table_properties:
                if prop.tag not in {w(tag) for tag in ('tblBorders', 'tblW', 'tblLayout')}:
                    self.document.loss('tableProperty:' + prop.tag.rsplit('}', 1)[-1])
        return table

    def _borders(self, node):
        if node is None:
            return {}
        result = {}
        for item in node:
            side = item.tag.rsplit('}', 1)[-1]
            if side not in {'top', 'bottom', 'left', 'right', 'insideH', 'insideV'}:
                self.document.loss('diagonalBorders')
                continue
            kind = attr(item, default='single')
            if kind not in {'single', 'double', 'dotted', 'dashed', 'nil', 'none'}:
                self.document.loss('borderStyles')
                kind = 'single'
            result[side] = dict(kind=kind, size=integer(attr(item, 'sz'), 4, minimum=0, maximum=1024),
                                colour=colour(attr(item, 'color')) or '000000')
        return result

    def _blocks(self, node):
        blocks = []
        for child in node:
            if child.tag == w('p'):
                blocks.append(self._paragraph(child))
            elif child.tag == w('tbl'):
                blocks.append(self._table(child))
            elif child.tag in {w('sdt'), w('sdtContent'), w('customXml')}:
                self.document.loss('contentControls')
                blocks.extend(self._blocks(child))
        return blocks

    def _page(self, node):
        page = dict(DEFAULT_PAGE)
        if node is None:
            return page
        size = node.find(w('pgSz'))
        margin = node.find(w('pgMar'))
        if size is not None:
            page.update(width=integer(attr(size, 'w'), page['width'], minimum=1),
                        height=integer(attr(size, 'h'), page['height'], minimum=1),
                        orientation=attr(size, 'orient', 'portrait'))
        if margin is not None:
            for key in ['left', 'right', 'top', 'bottom', 'header', 'footer', 'gutter']:
                page[key] = integer(attr(margin, key), page[key])
        for item in node:
            if item.tag not in {w(tag) for tag in ('pgSz', 'pgMar', 'headerReference', 'footerReference', 'type')}:
                self.document.loss('sectionProperty:' + item.tag.rsplit('}', 1)[-1])
        return page

    def read(self):
        root = self.trees.get('word/document.xml')
        require(root is not None and root.tag == w('document'), 'NOT_DOCX', 'Missing DOCX main document.')
        body = root.find(w('body'))
        require(body is not None, 'NOT_DOCX', 'Missing DOCX body.')
        section = Section()
        for child in body:
            if child.tag == w('sectPr'):
                section.page = self._page(child)
            elif child.tag in {w('p'), w('tbl')}:
                section.blocks.append(self._paragraph(child) if child.tag == w('p') else self._table(child))
                boundary = child.find('./' + w('pPr') + '/' + w('sectPr')) if child.tag == w('p') else None
                if boundary is not None:
                    section.page = self._page(boundary)
                    self.document.sections.append(section)
                    section = Section()
            else:
                self.document.loss('bodyElement:' + child.tag.rsplit('}', 1)[-1])
                section.blocks.extend(self._blocks(child))
        if section.blocks or not self.document.sections:
            self.document.sections.append(section)
        return self.document


def read_docx(payload):
    parts, trees = read_package(payload)
    document = Reader(parts, trees).read()
    for section in document.sections:
        section.blocks = split_page_breaks(section.blocks)
    require(sum(1 for section in document.sections for _ in paragraphs(section.blocks)) <= MAX_NODES,
            'LIMIT', 'Page breaks exceed the paragraph limit.')
    return document
