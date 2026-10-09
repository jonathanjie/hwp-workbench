"""Pure, bounded text operations. Short read references map to native node IDs.

Regex syntax is deliberately limited to literals, classes, escapes, dots and
anchors. No repetitions, groups, branches or backreferences are accepted.
"""
from __future__ import annotations

from itertools import islice
import re
import unicodedata

from .document import Document, TextNode, check_text
from .errors import WorkbenchError, require

MAX_PATTERN = 256
MAX_SEARCH_MATCHES = 1000
MAX_PATCH_OPERATIONS = 1000
CONTEXT_PADDING = 40
MAX_TABLE_CELLS = 100_000
MAX_TABLE_COLUMNS = 1000
MAX_GRID_WORK = 1_000_000


def _integer(value: int, name: str, minimum: int, maximum: int) -> None:
    require(type(value) is int and minimum <= value <= maximum,
            'BAD_ARGUMENTS', f'{name} must be an integer from {minimum} to {maximum}.')


def _pattern(query: str, regex: bool, case_sensitive: bool) -> re.Pattern:
    require(isinstance(query, str) and bool(query), 'BAD_ARGUMENTS', 'Supply nonempty search text.')
    require(type(regex) is bool and type(case_sensitive) is bool,
            'BAD_ARGUMENTS', 'regex and caseSensitive must be booleans.')
    if regex:
        require(len(query) <= MAX_PATTERN, 'REGEX_LIMIT', 'Regex patterns are limited to 256 characters.')
        in_class = False
        escaped = False
        for character in query:
            if escaped:
                require(not character.isdigit() and character not in {'g', 'N'},
                        'REGEX_UNSUPPORTED', 'Regex backreferences and named escapes are not supported.')
                escaped = False
            elif character == '\\':
                escaped = True
            elif in_class:
                if character == ']':
                    in_class = False
            elif character == '[':
                in_class = True
            else:
                require(character not in '()*+?{}|', 'REGEX_UNSUPPORTED',
                        'Regex repetitions, groups and branches are not supported.')
    else:
        require(len(query) <= 200_000, 'LIMIT', 'Search text exceeds 200,000 characters.')
    expression = query if regex else re.escape(query)
    try:
        return re.compile(expression, 0 if case_sensitive else re.IGNORECASE)
    except re.error as error:
        raise WorkbenchError('BAD_REGEX', str(error)) from error


def _text_ids(view: dict) -> list[str]:
    """Get cell text, excluding nested tables which have their own cells."""
    if view['type'] == 'text':
        return [view['node']]
    if view['type'] == 'table':
        return []
    return [node_id for child in view.get('children', []) for node_id in _text_ids(child)]


def _cell_span(cell: dict) -> tuple[int, int, bool]:
    width = cell.get('colspan', 1)
    height = cell.get('rowspan', 1)
    native_span = cell.get('nativeSpan')
    if native_span is None:
        return width, height, True
    try:
        native_width = int(native_span['columns'])
        native_height = int(native_span['rows'])
        if native_width < 1 or native_height < 1:
            raise ValueError('Nonpositive cell span.')
        return native_width, native_height, True
    except (ValueError, TypeError, KeyError):
        return width, height, False


def _cell_address(cell: dict, default_row: int) -> tuple[int, int, bool]:
    try:
        row = int(cell['address']['row'])
        column = int(cell['address']['column'])
        if row < 0 or column < 0:
            raise ValueError('Negative cell address.')
        return row, column, True
    except (ValueError, TypeError, KeyError):
        return default_row, 0, False


def _tables(doc: Document) -> list[list[dict]]:
    """Use native positions when present; otherwise infer an ordered span grid."""
    tables = []
    cell_count = 0
    grid_work = 0

    def visit(view):
        nonlocal cell_count, grid_work
        if view['type'] == 'table':
            cells = []
            occupied_until = {}
            mapping_valid = True
            rows = [child for child in view.get('children', []) if child['type'] == 'row']
            for row_index, row in enumerate(rows):
                column = 0
                for cell in row.get('children', []):
                    if cell['type'] != 'cell':
                        continue
                    cell_count += 1
                    require(cell_count <= MAX_TABLE_CELLS, 'LIMIT', 'Table view exceeds 100,000 cells.')
                    width, height, valid_span = _cell_span(cell)
                    mapping_valid = mapping_valid and valid_span
                    require(width <= MAX_TABLE_COLUMNS and height <= MAX_TABLE_CELLS,
                            'LIMIT', 'Native table span exceeds the bounded mapping limits.')
                    cell_row = row_index
                    if cell.get('address') is not None:
                        cell_row, column, valid_address = _cell_address(cell, row_index)
                        mapping_valid = mapping_valid and valid_address
                    else:
                        while any(occupied_until.get(col, 0) > cell_row for col in range(column, column + width)):
                            column += 1
                            grid_work += width
                            require(column < MAX_TABLE_COLUMNS and grid_work <= MAX_GRID_WORK,
                                    'LIMIT', 'Table grid exceeds the bounded mapping limits.')
                    require(cell_row < MAX_TABLE_CELLS and column + width <= MAX_TABLE_COLUMNS,
                            'LIMIT', 'Table addresses exceed 100,000 rows or 1000 columns.')
                    if any(occupied_until.get(col, 0) > cell_row for col in range(column, column + width)):
                        mapping_valid = False
                    cells.append({'row': cell_row, 'column': column, 'width': width,
                                  'height': height, 'nodes': _text_ids(cell)})
                    grid_work += width
                    require(grid_work <= MAX_GRID_WORK, 'LIMIT', 'Table grid mapping exceeds its work limit.')
                    for col in range(column, column + width):
                        occupied_until[col] = cell_row + height
                    column += width
            for cell in cells:
                cell['mappingValid'] = mapping_valid
            tables.append(cells)
        for child in view.get('children', []):
            visit(child)

    for section in doc.view:
        visit(section)
    return tables


def read(doc: Document, offset: int = 0, limit: int = 200) -> dict:
    _integer(offset, 'offset', 0, 100_000)
    _integer(limit, 'limit', 1, 1000)
    references = {}
    by_id = {}
    section_numbers = {}
    section_counts = {}
    for node in doc.nodes:
        section = section_numbers.setdefault(node.part, len(section_numbers))
        index = section_counts.get(node.part, 0)
        section_counts[node.part] = index + 1
        by_id[node.id] = f's{section}:t{index}'
    row_by_id = {}
    cell_by_id = {}
    for table_index, cells in enumerate(_tables(doc)):
        for cell_index, cell in enumerate(cells):
            for node_id in cell['nodes']:
                row_by_id[node_id] = (table_index, cell['row'])
                cell_by_id[node_id] = (table_index, cell_index)
    lines = []
    previous_row = None
    previous_cell = None
    stop = min(len(doc.nodes), offset + limit)
    for node in doc.nodes[offset:stop]:
        reference = by_id[node.id]
        references[reference] = node.id
        marker = '' if node.editable else '(read-only) '
        # Escape grid separators and keep native embedded line breaks on one line.
        text = node.text.replace('\\', '\\\\').replace('|', '\\|').replace('\r', '\\r').replace('\n', '\\n')
        line = f'[{reference}] {marker}{text}'
        row = row_by_id.get(node.id)
        if row is not None:
            if row == previous_row:
                if cell_by_id[node.id] == previous_cell:
                    lines[-1] = lines[-1][:-2] + f' {line} |'
                else:
                    lines[-1] += f' {line} |'
            else:
                lines.append(f'| {line} |')
        else:
            lines.append(line)
        previous_row = row
        previous_cell = cell_by_id.get(node.id)
    return {'format': doc.format, 'sha256': doc.hash, 'total': len(doc.nodes),
            'offset': offset, 'limit': limit, 'nextOffset': stop if stop < len(doc.nodes) else None,
            'text': '\n'.join(lines), 'references': references,
            'warnings': [warning['code'] for warning in doc.warnings]}


def search(doc: Document, query: str, regex: bool = False,
           case_sensitive: bool = False, limit: int = 100) -> dict:
    _integer(limit, 'limit', 1, MAX_SEARCH_MATCHES)
    pattern = _pattern(query, regex, case_sensitive)
    matches = []
    for node in doc.nodes:
        for match in pattern.finditer(node.text):
            start, end = match.span()
            context_start = max(0, start - CONTEXT_PADDING)
            context_end = min(end + CONTEXT_PADDING, start + 2 * CONTEXT_PADDING)
            matches.append({'nodeId': node.id, 'start': start, 'end': end,
                            'context': node.text[context_start:context_end],
                            'editable': node.editable})
            if len(matches) == limit:
                return {'format': doc.format, 'sha256': doc.hash, 'matches': matches, 'limitReached': True}
    return {'format': doc.format, 'sha256': doc.hash, 'matches': matches, 'limitReached': False}


def _patch(doc: Document, operations: list[dict]) -> dict:
    return {'schemaVersion': 1, 'baseSha256': doc.hash, 'operations': operations}


def _operation(node: TextNode, text: str) -> dict:
    return {'op': 'set_text', 'nodeId': node.id, 'expectedText': node.text, 'text': text}


def _change(node: TextNode, text: str) -> dict:
    return {'nodeId': node.id, 'before': node.text, 'after': text, 'part': node.part}


def _edit_error(doc: Document, node: TextNode, text: str) -> str | None:
    if doc.block_save:
        return 'SAVE_BLOCKED'
    if not node.editable:
        return 'UNSUPPORTED_ELEMENT'
    if doc.format == 'hwp' and len(text.encode('utf-16le')) != len(node.text.encode('utf-16le')):
        return 'HWP_LENGTH_CHANGE_BLOCKED'
    return None


def _replacement_text(text: str, matches: list[re.Match], replacement: str) -> str:
    removed_characters = sum(match.end() - match.start() for match in matches)
    result_length = len(text) - removed_characters + len(matches) * len(replacement)
    # Check before allocation: 1000 large replacements must not build a huge string.
    require(result_length <= 200_000, 'BAD_PATCH', 'Replacement text exceeds 200,000 characters in one node.')
    fragments = []
    previous_end = 0
    for match in matches:
        fragments.append(text[previous_end:match.start()])
        fragments.append(replacement)
        previous_end = match.end()
    fragments.append(text[previous_end:])
    return ''.join(fragments)


def replace(doc: Document, find: str, replace_with: str, regex: bool = False,
            case_sensitive: bool = True, node_ids: list[str] | None = None,
            limit: int = 1000) -> dict:
    _integer(limit, 'limit', 1, MAX_PATCH_OPERATIONS)
    check_text(replace_with)
    pattern = _pattern(find, regex, case_sensitive)
    if node_ids is not None:
        require(isinstance(node_ids, list) and len(node_ids) <= MAX_PATCH_OPERATIONS
                and all(isinstance(node_id, str) for node_id in node_ids),
                'BAD_ARGUMENTS', 'nodeIds must contain at most 1000 full node IDs.')
        for node_id in node_ids:
            require(node_id in doc.by_id, 'NODE_NOT_FOUND', 'Unknown node ID: ' + node_id)
    selected = set(node_ids) if node_ids is not None else None
    operations = []
    changes = []
    skipped = []
    remaining = limit
    for node in doc.nodes:
        if remaining == 0:
            break
        if selected is not None and node.id not in selected:
            continue
        matches = list(islice(pattern.finditer(node.text), remaining))
        if not matches:
            continue
        remaining -= len(matches)
        code = _edit_error(doc, node, node.text)
        if code is None and doc.format == 'hwp':
            removed_bytes = sum(len(match.group().encode('utf-16le')) for match in matches)
            added_bytes = len(matches) * len(replace_with.encode('utf-16le'))
            if removed_bytes != added_bytes:
                code = 'HWP_LENGTH_CHANGE_BLOCKED'
        if code:
            skipped.append({'nodeId': node.id, 'code': code,
                            'reason': 'Read-only text or protected document.' if code != 'HWP_LENGTH_CHANGE_BLOCKED'
                            else 'Replacement must keep the same UTF-16 code-unit length.'})
            continue
        after = _replacement_text(node.text, matches, replace_with)
        check_text(after)
        if after != node.text:
            operations.append(_operation(node, after))
            changes.append(_change(node, after))
    return {'patch': _patch(doc, operations), 'changes': changes, 'skipped': skipped}


def _normalise_label(label: str) -> str:
    label = ' '.join(unicodedata.normalize('NFC', label).split())
    # The authored bilingual fixtures use "Korean label / English label".
    label = label.split(' / ', 1)[0].rstrip(':： ').strip()
    hangul = '\u1100-\u11ff\u3130-\u318f\uac00-\ud7a3'
    return re.sub(f'(?<=[{hangul}]) +(?=[{hangul}])', '', label)


def fill(doc: Document, fields: dict[str, str]) -> dict:
    require(isinstance(fields, dict) and len(fields) <= MAX_PATCH_OPERATIONS
            and all(isinstance(label, str) and bool(_normalise_label(label)) for label in fields),
            'BAD_ARGUMENTS', 'fields must map at most 1000 nonempty labels to text values.')
    for value in fields.values():
        check_text(value)
    operations = []
    filled = []
    unresolved = []
    details = []
    used_targets = set()
    tables = _tables(doc) if doc.format == 'hwpx' else []
    labels = {}
    for cells in tables:
        for cell in cells:
            text = ''.join(doc.by_id[node_id].text for node_id in cell['nodes'])
            labels.setdefault(_normalise_label(text), []).append((cells, cell))
    for label, value in fields.items():
        code = None
        target = None
        candidates = labels.get(_normalise_label(label), [])
        if doc.format == 'hwp':
            code = 'HWP_TABLE_MAPPING_UNAVAILABLE'
        elif len(candidates) != 1:
            code = 'AMBIGUOUS_LABEL' if candidates else 'LABEL_NOT_FOUND'
        elif not candidates[0][1]['mappingValid']:
            code = 'TABLE_MAPPING_UNAVAILABLE'
        else:
            cells, label_cell = candidates[0]
            right = [cell for cell in cells if cell['row'] == label_cell['row']
                     and cell['column'] >= label_cell['column'] + label_cell['width']]
            below = [cell for cell in cells if cell['row'] == label_cell['row'] + label_cell['height']
                     and cell['column'] == label_cell['column']]
            for cell in sorted(right, key=lambda cell: cell['column']) + below:
                # Replacing one run of a multi-run value could leave old text behind.
                if len(cell['nodes']) != 1:
                    continue
                node = doc.by_id[cell['nodes'][0]]
                if _edit_error(doc, node, value) is None:
                    target = node
                    break
            if target is None:
                code = 'NO_EDITABLE_VALUE_CELL'
            elif target.id in used_targets:
                code = 'TARGET_CONFLICT'
        if code:
            unresolved.append(label)
            details.append({'label': label, 'code': code})
            continue
        used_targets.add(target.id)
        if target.text != value:
            operations.append(_operation(target, value))
        filled.append({'label': label, 'nodeId': target.id, 'before': target.text, 'after': value})
    return {'patch': _patch(doc, operations), 'filled': filled,
            'unresolved': unresolved, 'unresolvedDetails': details}


def set_text(doc: Document, node_id: str, text: str) -> dict:
    require(isinstance(node_id, str), 'BAD_ARGUMENTS', 'nodeId must be a full node ID.')
    require(node_id in doc.by_id, 'NODE_NOT_FOUND', 'Unknown node ID: ' + node_id)
    check_text(text)
    node = doc.by_id[node_id]
    code = _edit_error(doc, node, text)
    if code is not None:
        raise WorkbenchError(code, 'This node cannot accept the requested text.')
    return {'patch': _patch(doc, [_operation(node, text)]),
            'changes': [] if text == node.text else [_change(node, text)]}
