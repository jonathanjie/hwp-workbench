#!/usr/bin/env python3
"""Test pinned public bytes locally. Missing or skipped checks never pass."""
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'engine'))
from hwp_workbench.document import Document, sha
from hwp_workbench.errors import WorkbenchError
from hwp_workbench.independent import probe, invoke as bridge_invoke
from rhwp_probe import probe_rhwp


def equal_length_edit(document):
    for node in document.nodes:
        if not node.editable or not node.text or not all(ord(char) >= 32 for char in node.text):
            continue
        # Replace one BMP character only. This preserves UTF-16 length even if
        # the node also contains surrogate pairs (for example, emoji).
        for offset in range(len(node.text) - 1, -1, -1):
            if ord(node.text[offset]) <= 0xffff:
                replacement = '가' if node.text[offset] != '가' else '나'
                text = node.text[:offset] + replacement + node.text[offset + 1:]
                return node, text
    return None


def independent_check(payload, expected_text=None):
    result = probe_rhwp(payload)
    text = result.pop('text', None)
    if result['status'] == 'passed' and expected_text is not None:
        if expected_text not in text:
            result.update(status='failed', reason='Edited text was not found by rhwp.')
        else:
            result['editedTextFound'] = True
    return result


def probe_check(payload):
    try:
        observed = probe(payload)
        if observed.get('available') is False:
            return {'status': 'skipped', 'reason': observed['reason']}
        return {'status': 'passed', **observed}
    except WorkbenchError as error:
        return {'status': 'failed', 'error': error.as_dict()}


def anchor_agreement(document, observed):
    body = [node for node in document.nodes if node.text and node.anchor['charOffset'] is not None]
    result = {
        'checkedBodyNodes': len(body),
        'cellNodes': sum(node.anchor['inCell'] for node in document.nodes),
        'otherNestedNodes': sum(node.anchor['charOffset'] is None and not node.anchor['inCell']
                                for node in document.nodes),
        'scope': 'Top-level paragraph getTextRange, Unicode scalar offsets; nested text has no direct range.',
    }
    if observed['status'] != 'passed':
        return {**result, 'status': 'skipped', 'checkedBodyNodes': 0,
                'reason': observed.get('reason', 'rhwp probe failed; no independent ranges are available.')}
    if not body:
        return {**result, 'status': 'skipped', 'reason': 'No top-level body text nodes; nested text is outside this range API.'}
    paragraphs = {(section['index'], paragraph['index']): paragraph['text']
                  for section in observed['sections'] for paragraph in section['paragraphs']}
    missing = []
    for node in body:
        anchor = node.anchor
        text = paragraphs.get((anchor['section'], anchor['paragraph']))
        start = anchor['charOffset']
        # The probe retrieves the full paragraph with getTextRange. Python and
        # Rust both slice this text by Unicode scalars, rather than UTF-16 units.
        if text is None or text[start:start + len(node.text)] != node.text:
            missing.append(node.id)
    return {**result, 'status': 'failed' if missing else 'passed', 'missing': missing}


def length_changing_checks(document, output_directory, name):
    """Retain native and independent evidence separately for a real HWP edit."""
    node = next((node for node in document.nodes if node.editable and node.text), None)
    if node is None:
        reason = 'No existing printable text run for a length-changing edit.'
        return {check: {'status': 'skipped', 'reason': reason} for check in
                ('lengthChangingEdit', 'rhwpLengthChangingEdit', 'rhwpLengthChangingRender')}
    replacement = node.text + ' edited'
    document.patch({
        'schemaVersion': 1, 'baseSha256': document.hash,
        'operations': [{'op': 'set_text', 'nodeId': node.id,
                        'expectedText': node.text, 'text': replacement}],
    })
    saved = document.serialize(True)
    output_path = output_directory / ('length-edited-' + name)
    with output_path.open('xb') as output:
        output.write(saved)
    reopened = Document(output_path.read_bytes(), output_path.name)
    if reopened.by_id[node.id].text != replacement:
        raise ValueError('Length-changing text differs after engine reopen.')
    untouched = all(reopened.parts[part] == payload for part, payload in document.parts.items()
                    if part != node.part)
    if not untouched:
        raise ValueError('Length-changing edit changed an untouched stream.')
    checks = {'lengthChangingEdit': {
        'status': 'passed', 'nodeId': node.id,
        'utf16Delta': (len(replacement.encode('utf-16le')) - len(node.text.encode('utf-16le'))) // 2,
        'reopenedAndVerified': True, 'untouchedStreamsIdentical': untouched, 'sha256': sha(saved),
    }}
    try:
        observed = probe(saved)
        if observed.get('available') is False:
            skipped = {'status': 'skipped', 'reason': observed['reason']}
            checks['rhwpLengthChangingEdit'] = dict(skipped)
            checks['rhwpLengthChangingRender'] = dict(skipped)
            return checks
        paragraphs = [paragraph['text'] for section in observed['sections']
                      for paragraph in section['paragraphs']]
        method = 'paragraphs' if replacement in paragraphs else 'fullText'
        found = replacement in paragraphs or replacement in observed['fullText']
        checks['rhwpLengthChangingEdit'] = {
            'status': 'passed' if found and observed['pageCount'] >= 1 else 'failed',
            'engine': 'rhwp ' + observed['version'], 'pageCount': observed['pageCount'],
            'editedTextFound': found, 'method': method if found else 'missing',
        }
    except WorkbenchError as error:
        checks['rhwpLengthChangingEdit'] = {'status': 'failed', 'error': error.as_dict()}
    try:
        rendered = bridge_invoke(saved, 'render', page=0)
        if rendered.get('available') is False:
            checks['rhwpLengthChangingRender'] = {'status': 'skipped', 'reason': rendered['reason']}
        else:
            svg = rendered['svg'].encode('utf-8')
            checks['rhwpLengthChangingRender'] = {
                'status': 'passed' if '<svg' in rendered['svg'] else 'failed',
                'page': rendered['page'], 'svgBytes': len(svg), 'svgSha256': sha(svg),
                'scope': 'Bridge SVG with approximate Node text metrics; Hancom reflow unverified.',
            }
    except WorkbenchError as error:
        checks['rhwpLengthChangingRender'] = {'status': 'failed', 'error': error.as_dict()}
    return checks


def test_fixture(item, output_directory):
    path = ROOT / 'fixtures/public' / item['name']
    result = {'name': item['name'], 'hancomCertified': False, 'checks': {}}
    if not path.is_file():
        result.update(status='not-run', reason='Fixture not downloaded.')
        return result
    try:
        raw = path.read_bytes()
        blob = hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()
        if blob != item['blobSha1'] or len(raw) != item['bytes']:
            raise ValueError('Fixture identity/size differs from manifest.')
        document = Document(raw, path.name)
        result.update(
            sourceSha256=sha(raw),
            textNodes=len(document.nodes),
            warnings=[warning['code'] for warning in document.warnings],
        )
        checks = result['checks']
        checks['rhwpProbe'] = probe_check(raw)
        checks['anchorAgreement'] = anchor_agreement(document, checks['rhwpProbe'])
        noop_path = output_directory / ('noop-' + path.name)
        with noop_path.open('xb') as output:
            output.write(document.serialize())
        noop = noop_path.read_bytes()
        if noop != raw:
            raise ValueError('No-op save changed source bytes.')
        Document(noop, noop_path.name)
        checks['noop'] = {'status': 'passed', 'byteIdentical': True}
        checks['rhwpNoop'] = independent_check(noop)
        candidate = equal_length_edit(document)
        if candidate is None:
            checks['edit'] = {'status': 'skipped', 'reason': 'No printable body text.'}
            checks['rhwpEdit'] = {'status': 'skipped', 'reason': 'No edited output exists because the source has no editable text.'}
        else:
            node, text = candidate
            if len(text.encode('utf-16le')) != len(node.text.encode('utf-16le')):
                raise ValueError('Corpus edit must preserve UTF-16 length.')
            document.patch({
                'schemaVersion': 1,
                'baseSha256': document.hash,
                'operations': [{'op': 'set_text', 'nodeId': node.id, 'expectedText': node.text, 'text': text}],
            })
            edited_path = output_directory / ('edited-' + path.name)
            with edited_path.open('xb') as output:
                output.write(document.serialize(True))
            saved = edited_path.read_bytes()
            reopened = Document(saved, edited_path.name)
            if reopened.by_id[node.id].text != text:
                raise ValueError('Edited text differs after Python reopen.')
            checks['edit'] = {'status': 'passed', 'equalUtf16Length': True, 'reopenedAndVerified': True, 'sha256': sha(saved)}
            checks['rhwpEdit'] = independent_check(saved, text)
            checks['rhwpEditedProbe'] = probe_check(saved)
            checks['editedAnchorAgreement'] = anchor_agreement(reopened, checks['rhwpEditedProbe'])
        if item['name'] == 'pyhwp-aligns.hwp':
            checks.update(length_changing_checks(Document(raw, path.name), output_directory, path.name))
        statuses = {check['status'] for check in checks.values()}
        result['status'] = 'failed' if 'failed' in statuses else 'partial' if 'skipped' in statuses else 'passed'
    except Exception as error:
        result.update(status='failed', error=str(error))
    return result


def main():
    manifest = json.loads((ROOT / 'fixtures/public-manifest.json').read_text())
    scratch = ROOT / 'work'
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='public-corpus-', dir=scratch) as directory:
        results = [test_fixture(item, Path(directory)) for item in manifest['fixtures']]
    report = {
        'suite': 'public corpus: native save/reopen, length-changing HWP edit, independent rhwp probe/render and body anchors',
        'generatedAt': datetime.now(timezone.utc).isoformat(),
        'host': platform.node(),
        'platform': platform.platform(),
        'machine': platform.machine(),
        'python': platform.python_version(),
        'success': all(result['status'] == 'passed' for result in results),
        'failures': sum(result['status'] in {'failed', 'not-run'} for result in results),
        'skipped': sum(check['status'] == 'skipped' for result in results for check in result['checks'].values()),
        'results': results,
        'notTested': ['Hancom reopen and layout comparison', 'Font metrics and visual fidelity'],
    }
    (ROOT / 'reports/public-corpus-results.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    # Partial evidence remains explicit even when all checks that could run passed.
    return 0 if report['failures'] == 0 else 1


if __name__ == '__main__':
    sys.exit(main())
