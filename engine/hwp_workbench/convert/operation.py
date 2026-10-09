"""Shared conversion routes, loss evidence and exclusive output publication."""
import base64
from collections import Counter
import difflib
from pathlib import Path
import unicodedata
import struct

from .. import independent
from ..cfb import CompoundFile, MAGIC
from ..document import Document, sha
from ..errors import MAX_FILE, WorkbenchError, require
from . import libreoffice
from .docx_read import read_docx
from .docx_write import write_docx
from .hwpx_read_rich import read_hwpx
from .hwpx_write import write_hwpx
from .package import read_package

TARGETS = {'hwp', 'hwpx', 'docx'}
MAX_TEXT_DIFF_WORK = 1_000_000
MAX_TEXT_DIFF_DETAILS = 1000


def detect(payload):
    if payload.startswith(MAGIC):
        streams = CompoundFile(payload).streams
        if 'FileHeader' in streams and streams['FileHeader'].startswith(b'HWP Document File'):
            return 'hwp'
        require('WordDocument' in streams, 'UNSUPPORTED_FORMAT', 'Unsupported compound document.')
        return 'doc'
    if payload.startswith(b'PK'):
        parts, _ = read_package(payload)
        if parts.get('mimetype', b'').strip() == b'application/hwp+zip':
            return 'hwpx'
        if parts.get('mimetype', b'').strip() == b'application/vnd.oasis.opendocument.text':
            return 'odt'
        require('word/document.xml' in parts and '[Content_Types].xml' in parts, 'UNSUPPORTED_FORMAT', 'Unsupported ZIP document.')
        return 'docx'
    if payload.lstrip().startswith(b'{\\rtf'):
        return 'rtf'
    raise WorkbenchError('UNSUPPORTED_FORMAT', 'Conversion accepts HWP 5, HWPX, DOCX, DOC, ODT or RTF.')


def native_texts(payload):
    document = Document(payload)
    if document.format == 'hwp':
        result = []
        for section in document.hwp_sections.values():
            for paragraph in section.paragraphs.values():
                text_parts = []
                table_carrier = False
                for index in paragraph.children:
                    record = section.records[index]
                    if record.tag == 71 and record.data[:4][::-1] == b'tbl ':
                        table_carrier = True
                    if record.tag != 67:
                        continue
                    offset = 0
                    data = record.data
                    while offset < len(data):
                        code = struct.unpack_from('<H', data, offset)[0]
                        if code < 32:
                            width = 16 if (1 <= code <= 9 or 11 <= code <= 12 or 14 <= code <= 23) else 2
                            require(offset + width <= len(data), 'BAD_HWP_TEXT', 'Truncated native text control.')
                            if code in {9, 10, 24, 25, 30, 31}:
                                text_parts.append({9: '\t', 10: '\n', 24: '\u00ad', 25: ' ', 30: '\u00a0', 31: '\u2007'}[code])
                            offset += width
                        else:
                            start = offset
                            while offset < len(data) and struct.unpack_from('<H', data, offset)[0] >= 32:
                                offset += 2
                            text_parts.append(data[start:offset].decode('utf-16le'))
                text = ''.join(text_parts)
                if not table_carrier or text:
                    result.append(text)
        return result
    grouped = {}
    for node in document.nodes:
        grouped.setdefault((node.part, node.paragraph), []).append(node.text)
    result = []
    for part in sorted({node.part for node in document.nodes}, key=lambda name: int(name.rsplit('section', 1)[1].split('.')[0])):
        root = document._xml(document.parts[part])
        paragraphs = [item for item in root.iter() if item.tag == '{http://www.hancom.co.kr/hwpml/2011/paragraph}p']
        for index, paragraph in enumerate(paragraphs, 1):
            texts = grouped.get((part, index), [])
            text = ''.join(texts)
            has_table = any(item.tag.endswith('}tbl') for run in paragraph if run.tag.endswith('}run') for item in run)
            if has_table and not text:
                continue
            # A real empty paragraph can have no text element at all.
            result.append(text)
    return result


def text_check(before, after, *, source='source document'):
    def normalise(text):
        return ' '.join(unicodedata.normalize('NFC', text).split())
    source_texts = [normalise(text) for text in before]
    output_texts = [normalise(text) for text in after]
    missing, changed, added = [], [], []
    if len(before) * len(after) <= MAX_TEXT_DIFF_WORK:
        matcher = difflib.SequenceMatcher(a=source_texts, b=output_texts, autojunk=False)
        for operation, a_start, a_end, b_start, b_end in matcher.get_opcodes():
            if operation == 'equal':
                continue
            paired = min(a_end - a_start, b_end - b_start) if operation == 'replace' else 0
            changed.extend({'sourceIndex': i, 'outputIndex': j, 'before': before[i][:120], 'after': after[j][:120]}
                           for i, j in zip(range(a_start, a_start + paired), range(b_start, b_start + paired)))
            missing.extend({'sourceIndex': i, 'text': before[i][:120]} for i in range(a_start + paired, a_end))
            added.extend({'outputIndex': j, 'text': after[j][:120]} for j in range(b_start + paired, b_end))
        comparison = 'aligned paragraph sequence'
    else:
        # Bound adversarial repeated-paragraph comparisons. Counter difference
        # establishes multiplicity; index differences still show reordering.
        remaining = Counter(output_texts)
        for index, text in enumerate(source_texts):
            if remaining[text]:
                remaining[text] -= 1
            else:
                missing.append({'sourceIndex': index, 'text': before[index][:120]})
        changed = [{'sourceIndex': i, 'outputIndex': i, 'before': before[i][:120], 'after': after[i][:120]}
                   for i in range(min(len(before), len(after))) if source_texts[i] != output_texts[i]]
        added = [{'outputIndex': i, 'text': after[i][:120]} for i in range(len(before), len(after))]
        comparison = 'bounded multiplicity and index comparison'
    return {'performed': True, 'source': source, 'scope': 'readable paragraphs; unsupported object semantics and layout are not checked',
            'normalisation': 'Unicode NFC and whitespace collapse within each paragraph', 'comparison': comparison,
            'missingParagraphs': missing[:MAX_TEXT_DIFF_DETAILS], 'changedParagraphs': changed[:MAX_TEXT_DIFF_DETAILS],
            'addedParagraphs': added[:MAX_TEXT_DIFF_DETAILS], 'missingParagraphCount': len(missing),
            'changedParagraphCount': len(changed), 'addedParagraphCount': len(added),
            'detailsTruncated': any(len(items) > MAX_TEXT_DIFF_DETAILS for items in (missing, changed, added)),
            'sourceParagraphs': len(before), 'outputParagraphs': len(after),
            'sourceCharacters': sum(map(len, before)), 'outputCharacters': sum(map(len, after)),
            'matches': source_texts == output_texts}


def _export(payload, target, reports):
    exported = independent.invoke(payload, 'export', format=target)
    if exported.get('available') is False:
        reports.append({'engine': 'rhwp'} | exported)
        return None
    encoded = exported.get('bytesBase64')
    require(isinstance(encoded, str) and len(encoded) <= ((MAX_FILE + 2) // 3) * 4,
            'INDEPENDENT_PROTOCOL', 'Invalid or oversized rhwp export.')
    require(isinstance(exported.get('lossReport'), (dict, list)), 'INDEPENDENT_PROTOCOL', 'rhwp export omitted its loss report.')
    try:
        result = base64.b64decode(encoded, validate=True)
    except ValueError as error:
        raise WorkbenchError('INDEPENDENT_PROTOCOL', 'Invalid rhwp export bytes.') from error
    if target == 'hwpx':
        read_package(result)
    require(Document(result).format == target, 'VERIFY_FAILED', 'rhwp exported the wrong format.')
    reports.append({'engine': 'rhwp', 'to': target, 'lossReport': exported['lossReport'],
                    'scope': 'rhwp export losses; does not cover Python import losses or certify layout'})
    return result


def _mapper_report(source, target, model):
    return {'engine': 'Python direct mapper', 'from': source, 'to': target,
            'unsupported': dict(model.unsupported), 'layoutCertified': False,
            'scope': 'supported properties only; fonts, pagination and Hancom layout are unqualified'}


def _native_unsupported(payload):
    document = Document(payload)
    if document.format == 'hwpx':
        return read_hwpx(payload).unsupported
    features = {b'head': 'headers', b'foot': 'footers', b'fn  ': 'footnotes', b'en  ': 'endnotes',
                b'%hlk': 'fields', b'%dte': 'fields', b'%pag': 'fields', b'eqed': 'equations',
                b'gso ': 'shapes', b'tcmt': 'comments'}
    counts = Counter()
    if document.block_save:
        counts['activeOrSignedContent'] += 1
    for section in document.hwp_sections.values():
        for item in section.records:
            if item.tag == 71 and len(item.data) >= 4:
                control = item.data[:4][::-1]
                if control in features:
                    counts[features[control]] += 1
                elif control not in {b'secd', b'cold', b'tbl ', b'atno', b'nwno'}:
                    counts['nativeControl:' + control.decode('ascii', errors='replace')] += 1
    return dict(counts)


def convert(workspace, args):
    require(set(args) <= {'path', 'to', 'output', 'acknowledgeRisks'}, 'BAD_ARGUMENTS', 'Unknown conversion argument.')
    target = args.get('to')
    require(isinstance(target, str) and target in TARGETS, 'BAD_ARGUMENTS', 'to must be hwp, hwpx or docx.')
    require('acknowledgeRisks' not in args or type(args['acknowledgeRisks']) is bool,
            'BAD_ARGUMENTS', 'acknowledgeRisks must be a boolean.')
    output = args.get('output')
    if 'output' in args:
        require(isinstance(output, str) and Path(output).suffix.lower() == '.' + target,
                'OUTPUT_FORMAT', 'Conversion output must use the requested target extension.')
        workspace.relative_path(output)  # Check confinement even for a dry run.
    path = args.get('path')
    original = workspace.read(path)
    source = detect(original)
    reports, route = [], []
    unsupported = {}
    result = {'from': source, 'to': target, 'route': route, 'engineReports': reports,
              'textCheck': {'performed': False}, 'unsupported': unsupported, 'layoutCertified': False}
    content, current = original, source
    check_source = 'source document'
    if source in {'doc', 'odt', 'rtf'}:
        route.append(f'{source} → docx (LibreOffice)')
        content, report = libreoffice.convert(original, source, 'docx')
        reports.append(report)
        if content is None:
            return result | {'available': False}
        current = 'docx'
        check_source = 'LibreOffice DOCX intermediate; original legacy import losses cannot be independently measured'
        unsupported['legacyImportUnverified'] = 1
    if current == 'docx':
        model = read_docx(content)
        before = model.paragraph_texts()
    else:
        unsupported.update(_native_unsupported(content))
        before = native_texts(content)
    if current == target:
        route.append(f'{current} → {target} (validated byte copy)')
        reports.append({'engine': 'byte copy', 'lossReport': {'losses': []}, 'originalBytesRetained': True})
        # Copies retain unsupported payloads; legacy import remains unverified.
        unsupported.clear()
        if source in {'doc', 'odt', 'rtf'}:
            unsupported['legacyImportUnverified'] = 1
    else:
        if current == 'hwp':
            route.append('hwp → hwpx (rhwp)')
            content = _export(content, 'hwpx', reports)
            if content is None:
                return result | {'available': False}
            current = 'hwpx'
        if current == 'docx':
            route.append('docx → hwpx (Python direct mapper)')
            unsupported.update(model.unsupported)
            content = write_hwpx(model)
            reports.append(_mapper_report('docx', 'hwpx', model))
            current = 'hwpx'
        if target == 'docx':
            route.append('hwpx → docx (Python direct mapper)')
            model = read_hwpx(content)
            # Counts for source native features may also appear in the pivot.
            # Use their maximum, rather than counting the same object twice.
            for feature, count in model.unsupported.items():
                unsupported[feature] = max(unsupported.get(feature, 0), count)
            content = write_docx(model)
            reports.append(_mapper_report('hwpx', 'docx', model))
        elif target == 'hwp':
            route.append('hwpx → hwp (rhwp)')
            content = _export(content, 'hwp', reports)
            if content is None:
                return result | {'available': False}
    after = read_docx(content).paragraph_texts() if target == 'docx' else native_texts(content)
    result['textCheck'] = text_check(before, after, source=check_source)
    result['available'] = True
    if output is not None and args.get('acknowledgeRisks') is True:
        require(sha(workspace.read(path)) == sha(original), 'STALE_DOCUMENT', 'Source changed during conversion; output was not published.')
        workspace.write_new(output, content)
        result.update(output=output, outputSha256=sha(content))
    return result
