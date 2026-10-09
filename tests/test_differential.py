"""Check exported fixtures and guard the differential oracle against false passes."""
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def script(name):
    path = ROOT / 'scripts' / (name + '.py')
    if not path.exists():
        raise AssertionError('Required script is missing: ' + str(path))
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class DifferentialTests(unittest.TestCase):
    def test_exported_fixtures_match_builders(self):
        exporter = script('export-test-fixtures')
        manifest = json.loads((ROOT / 'tests/data/manifest.json').read_text())
        self.assertTrue(manifest)
        self.assertEqual(len({item['name'] for item in manifest}), len(manifest))
        for item in manifest:
            with self.subTest(name=item['name']):
                payload = exporter.rebuild(item)
                self.assertEqual(payload, (ROOT / 'tests/data' / item['name']).read_bytes())
                self.assertEqual(exporter.digest(payload), item['sha256'])

    def test_semantic_documents_keep_order_compression_and_payload(self):
        runner = script('differential')
        def package(order, time, compression, text=b'original'):
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, 'w') as archive:
                for name in order:
                    member = zipfile.ZipInfo(name, time)
                    member.compress_type = compression
                    archive.writestr(member, text)
            return buffer.getvalue()
        first = package(['a', 'b'], (2026, 1, 1, 0, 0, 0), zipfile.ZIP_DEFLATED)
        same = package(['a', 'b'], (2026, 2, 1, 0, 0, 0), zipfile.ZIP_DEFLATED)
        self.assertEqual(runner.document_value(first), runner.document_value(same))
        for other in [package(['b', 'a'], (2026, 1, 1, 0, 0, 0), zipfile.ZIP_DEFLATED),
                      package(['a', 'b'], (2026, 1, 1, 0, 0, 0), zipfile.ZIP_STORED),
                      package(['a', 'b'], (2026, 1, 1, 0, 0, 0), zipfile.ZIP_DEFLATED, b'changed')]:
            self.assertNotEqual(runner.document_value(first), runner.document_value(other))

    def test_exact_comparison_preserves_types_and_unapproved_paths(self):
        runner = script('differential')
        differences = runner.differences({'a': True, 'b': [1]}, {'a': 1, 'b': [1, 2]})
        self.assertEqual({item['path'] for item in differences}, {'/a', '/b/1'})
        self.assertFalse(runner.is_approved('case', '/a', []))
        self.assertTrue(runner.is_approved('case', '/a', [
            {'case': 'case', 'path': '/a', 'reason': 'Reviewed type change', 'approvedBy': 'reviewer'}]))
        self.assertFalse(runner.is_approved('case', '/a/nested', [
            {'case': 'case', 'path': '/a', 'reason': 'Reviewed', 'approvedBy': 'reviewer'}]))

    def test_normalisation_does_not_hide_source_hashes_text_or_hash_errors(self):
        runner = script('differential')
        with tempfile.TemporaryDirectory() as folder:
            normal = runner.Normalizer(Path(folder), {})
            value = {'sha256': 'a' * 64, 'baseSha256': 'b' * 64,
                     'text': '20261010T100000Z-abcdef',
                     'error': {'message': 'bad ' + 'a' * 64}, 'createdAt': '2026-10-10T10:00:00Z'}
            observed = normal.value(value)
            self.assertEqual(observed['sha256'], value['sha256'])
            self.assertEqual(observed['baseSha256'], value['baseSha256'])
            self.assertEqual(observed['text'], value['text'])
            self.assertEqual(observed['error'], value['error'])
            self.assertNotEqual(observed['createdAt'], value['createdAt'])

    def test_cases_cover_rpc_and_mcp_contracts(self):
        from hwp_workbench.mcp import TOOLS
        runner = script('differential')
        cases = json.loads((ROOT / 'tests/contract/cases.json').read_text())
        self.assertEqual(len({case['id'] for case in cases}), len(cases))
        methods = {'capabilities', 'inspect', 'extract', 'validate', 'read', 'search', 'replace',
                   'fill', 'set', 'plan', 'apply', 'save', 'preview', 'format', 'propose', 'session', 'convert'}
        for extension in ['hwp', 'hwpx']:
            selected = [case for case in cases if case.get('format') == extension]
            covered = {step['method'] for case in selected for step in case['steps'] if step['kind'] == 'rpc'}
            self.assertEqual(covered & methods, methods)
        tools = {step['message']['params']['name'] for case in cases for step in case['steps']
                 if step['kind'] == 'mcp' and step['message'].get('method') == 'tools/call'}
        self.assertEqual({name for name in tools if name.startswith('hwp_')}, {tool['name'] for tool in TOOLS})
        runner.validate_cases(cases)

    def test_preview_hash_is_bound_to_semantic_bytes(self):
        import base64
        runner = script('differential')
        def preview(time):
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, 'w') as archive:
                archive.writestr(zipfile.ZipInfo('part', time), b'body')
            payload = buffer.getvalue()
            return {'preview': {'stdout': {'sha256': runner.sha(payload),
                                         'bytesBase64': base64.b64encode(payload).decode()}}}
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            first = runner.Normalizer(root, {}).output(preview((2026, 1, 1, 0, 0, 0)), {})
            second = runner.Normalizer(root, {}).output(preview((2026, 2, 1, 0, 0, 0)), {})
            self.assertEqual(first, second)

    def test_multiple_proposals_keep_distinct_identity_and_contents(self):
        runner = script('differential')
        with tempfile.TemporaryDirectory() as folder:
            normal = runner.Normalizer(Path(folder), {})
            results = {'first': {'stdout': {'proposal': 'form.hwpx.20260101T000000Z-aaaaaa.json'}},
                       'second': {'stdout': {'proposal': 'form.hwpx.20260101T000000Z-bbbbbb.json'}}}
            observed = normal.output(results, {})
            self.assertNotEqual(observed['steps']['first'], observed['steps']['second'])

    def test_case_rejects_candidate_source_rewrite(self):
        import sys
        runner = script('differential')
        with tempfile.TemporaryDirectory() as folder:
            candidate = Path(folder) / 'candidate.py'
            candidate.write_text("import json, pathlib, sys\n"
                                 "root = pathlib.Path(sys.argv[-1])\n"
                                 "request = json.load(sys.stdin)\n"
                                 "(root / 'source.hwpx').write_bytes(b'changed')\n"
                                 "print(json.dumps({'version': 'candidate'}))\n")
            case = {'id': 'source-rewrite', 'purpose': 'Keep source bytes',
                    'seeds': [{'source': 'fixtures/generated/review-form.hwpx', 'path': 'source.hwpx'}],
                    'steps': [{'id': 'call', 'kind': 'rpc', 'method': 'capabilities', 'args': {},
                               'expect': {'exitCode': 0}}]}
            with self.assertRaisesRegex(runner.CaseError, 'Source bytes changed'):
                runner.execute(case, [sys.executable, str(candidate)])

    def test_cfb_comparison_decodes_body_and_preserves_opaque_streams(self):
        import zlib
        from fixtures import make_hwp
        from hwp_workbench.cfb import CompoundFile
        runner = script('differential')
        original = make_hwp()
        source = CompoundFile(original)
        changes = {}
        for name in ['DocInfo', 'BodyText/Section0']:
            decoded = zlib.decompress(source.streams[name], -15)
            compressor = zlib.compressobj(level=1, wbits=-15)
            changes[name] = compressor.compress(decoded) + compressor.flush()
        rewritten = source.rewrite(changes)
        self.assertEqual(runner.document_value(original), runner.document_value(rewritten))
        changed = source.rewrite({'BinData/opaque.bin': b'changed'})
        self.assertNotEqual(runner.document_value(original), runner.document_value(changed))

    def test_package_entry_names_are_never_normalised(self):
        runner = script('differential')
        with tempfile.TemporaryDirectory() as folder:
            normal = runner.Normalizer(Path(folder), {})
            value = {'container': 'zip', 'entries': [
                {'name': 'form.hwpx.20261010T100000Z-abcdef.json', 'compression': 0, 'bytesBase64': 'eA=='}]}
            self.assertEqual(normal.value(value), value)

    def test_produced_stream_hashes_follow_decompressed_bytes(self):
        from fixtures import make_hwp
        from hwp_workbench.cfb import CompoundFile
        import zlib
        runner = script('differential')
        original = make_hwp()
        source = CompoundFile(original)
        before = zlib.decompress(source.streams['BodyText/Section0'], -15)
        changed = before.replace('마리나체인'.encode('utf-16le'), '새로운기관명'.encode('utf-16le'))
        outputs = []
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for level in [1, 9]:
                compressor = zlib.compressobj(level=level, wbits=-15)
                stream = compressor.compress(changed) + compressor.flush()
                payload = source.rewrite({'BodyText/Section0': stream})
                (root / 'result.hwp').write_bytes(payload)
                normal = runner.Normalizer(root, [runner.sha(original)])
                normal.initial_hashes.update(runner.sha(item) for item in source.streams.values())
                observed = normal.output({'inspect': {'stdout': {'parts': [
                    {'name': 'BodyText/Section0', 'sha256': runner.sha(stream)}]}}}, runner.inventory(root))
                outputs.append(observed)
            self.assertEqual(outputs[0], outputs[1])

    def test_comparator_bounds_archive_expansion(self):
        from convert_fixtures import package
        from hwp_workbench.errors import WorkbenchError
        runner = script('differential')
        payload = package({'large.bin': b'A' * (33 * 1024 * 1024)})
        with self.assertRaises(WorkbenchError):
            runner.document_value(payload)

    def test_case_rejects_preview_hash_that_does_not_match_bytes(self):
        import sys
        runner = script('differential')
        with tempfile.TemporaryDirectory() as folder:
            candidate = Path(folder) / 'candidate.py'
            candidate.write_text("import json, sys\njson.load(sys.stdin)\n"
                                 "print(json.dumps({'sha256': '0' * 64, 'bytesBase64': 'eA=='}))\n")
            case = {'id': 'invalid-preview-hash', 'purpose': 'Bind a digest to preview bytes',
                    'steps': [{'id': 'preview', 'kind': 'rpc', 'method': 'preview', 'args': {},
                               'expect': {'exitCode': 0}}]}
            with self.assertRaisesRegex(runner.CaseError, 'digest does not match'):
                runner.execute(case, [sys.executable, str(candidate)])
