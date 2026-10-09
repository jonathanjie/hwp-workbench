"""Read native HWPX properties without conversion defaults or document writes."""
from math import isfinite

from .document import HP, HH, local
from .errors import require
from .convert.hwpx_read_rich import find, colour


def number(value, divisor=1):
    try:
        result = float(value) / divisor if value is not None else None
        return result if result is not None and isfinite(result) else None
    except (ValueError, TypeError, OverflowError):
        return None


def read_format(doc, node_id=None):
    result = {'character': {}, 'paragraph': {}, 'page': {}, 'style': {},
              'fonts': getattr(doc, 'fonts', []), 'source': 'native-header'}
    selected = next((node for node in doc.nodes if node.id == node_id), None)
    require(node_id is None or selected is not None, 'BAD_ARGUMENTS', 'Unknown text node.')
    if doc.format != 'hwpx':
        return result
    header = doc._xml(doc.parts['Contents/header.xml'])
    selected = selected or next(iter(doc.nodes), None)
    if selected is None:
        return result
    body = doc._xml(doc.parts[selected.part])
    texts = [item for item in body.iter() if item.tag == '{' + HP + '}t']
    nodes = [node for node in doc.nodes if node.part == selected.part]
    text = texts[nodes.index(selected)]
    parents = {child: parent for parent in body.iter() for child in parent}
    ancestors = []
    current = text
    while current in parents:
        current = parents[current]
        ancestors.append(current)
    run = next((item for item in ancestors if local(item.tag) == 'run'), None)
    para = next((item for item in ancestors if local(item.tag) == 'p'), None)
    references = {kind: {item.get('id'): item for item in header.iter('{' + HH + '}' + kind)}
                  for kind in ['charPr', 'paraPr', 'style']}
    char = references['charPr'].get(run.get('charPrIDRef')) if run is not None else None
    if char is not None:
        fonts = {}
        for face in header.iter('{' + HH + '}fontface'):
            if face.get('lang') == 'HANGUL':
                fonts.update({font.get('id'): font.get('face') for font in face})
        ref = find(char, HH, 'fontRef')
        ratio = find(char, HH, 'ratio')
        spacing = find(char, HH, 'spacing')
        underline = find(char, HH, 'underline')
        result['character'] = {
            'font': fonts.get(ref.get('hangul')) if ref is not None else None,
            'size': number(char.get('height'), 100),
            'widthRatio': number(ratio.get('hangul')) if ratio is not None else None,
            'letterSpacing': number(spacing.get('hangul')) if spacing is not None else None,
            'bold': find(char, HH, 'bold') is not None,
            'italic': find(char, HH, 'italic') is not None,
            'underline': underline.get('type') != 'NONE' if underline is not None else False,
            'colour': colour(char.get('textColor'))}
    prop = references['paraPr'].get(para.get('paraPrIDRef')) if para is not None else None
    if prop is not None:
        align = find(prop, HH, 'align')
        default = prop.find('.//{' + HP + '}default')
        scope = default if default is not None else prop
        margin = scope.find('.//{' + HH + '}margin')
        spacing = scope.find('.//{' + HH + '}lineSpacing')
        result['paragraph']['alignment'] = align.get('horizontal') if align is not None else None
        if margin is not None:
            for item in margin:
                key = {'intent': 'indent', 'left': 'left', 'right': 'right', 'prev': 'before', 'next': 'after'}.get(local(item.tag))
                if key and item.get('unit') == 'HWPUNIT':
                    result['paragraph'][key] = number(item.get('value'), 100)
        if spacing is not None:
            value = number(spacing.get('value'))
            unit = spacing.get('type')
            if value is not None and unit == 'PERCENT':
                result['paragraph']['lineSpacing'] = f'{value:g}%'
            elif value is not None and unit in {'FIXED', 'AT_LEAST'}:
                result['paragraph']['lineSpacing'] = f'{value / 100:g} pt'
    style = references['style'].get(para.get('styleIDRef')) if para is not None else None
    result['style']['name'] = style.get('name') if style is not None else None
    page = next(body.iter('{' + HP + '}pagePr'), None)
    if page is not None:
        result['page'] = {key: number(page.get(key), 7200 / 25.4) for key in ['width', 'height']}
        result['page']['orientation'] = {'WIDELY': 'landscape', 'NARROWLY': 'portrait'}.get(page.get('landscape'))
        margin = find(page, HP, 'margin')
        if margin is not None:
            result['page'].update({key: number(margin.get(key), 7200 / 25.4) for key in ['left', 'right', 'top', 'bottom', 'header', 'footer', 'gutter']})
    return result
