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
        'suite': 'public corpus: native save/reopen and independent rhwp render',
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
