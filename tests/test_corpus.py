"""Regression checks for independent qualification and honest corpus results."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from fixtures import cfb_from_streams, make_hwp, make_hwpx, record
from hwp_workbench.cfb import CompoundFile
from hwp_workbench.document import Document

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from rhwp_probe import probe_rhwp

spec = importlib.util.spec_from_file_location('public_corpus', ROOT / 'scripts/test-public-corpus.py')
corpus = importlib.util.module_from_spec(spec)
spec.loader.exec_module(corpus)


class CorpusTests(unittest.TestCase):
    def test_empty_runs_do_not_count_as_body_anchor_evidence(self):
        streams = dict(CompoundFile(make_hwp(False)).streams)
        streams['BodyText/Section0'] = record(66, b'\0' * 22) + record(67, b'\r\0', 1)
        document = Document(cfb_from_streams(streams))
        observed = {'status': 'passed', 'sections': [{'index': 0, 'paragraphs': [{'index': 0, 'text': ''}]}]}
        result = corpus.anchor_agreement(document, observed)
        self.assertEqual(result['status'], 'skipped')
        self.assertEqual(result['checkedBodyNodes'], 0)

    def test_missing_node_is_skipped(self):
        with patch('rhwp_probe.shutil.which', return_value=None):
            result = probe_rhwp(make_hwp())
        self.assertEqual(result['status'], 'skipped')
        self.assertIn('Node.js', result['reason'])

    def test_length_changing_hwp_checks_save_reopen_and_render(self):
        from hwp_workbench import independent
        observed = independent.probe(make_hwp())
        if observed.get('available') is False:
            self.skipTest(observed['reason'])
        document = Document(make_hwp())
        with tempfile.TemporaryDirectory(dir=ROOT / 'work') as directory:
            checks = corpus.length_changing_checks(document, Path(directory), 'synthetic.hwp')
            saved = Document((Path(directory) / 'length-edited-synthetic.hwp').read_bytes())
        self.assertEqual(checks['lengthChangingEdit']['status'], 'passed')
        self.assertGreater(checks['lengthChangingEdit']['utf16Delta'], 0)
        self.assertTrue(checks['lengthChangingEdit']['untouchedStreamsIdentical'])
        self.assertTrue(saved.nodes[0].text.endswith(' edited'))
        self.assertEqual(checks['rhwpLengthChangingEdit']['status'], 'passed')
        self.assertEqual(checks['rhwpLengthChangingRender']['status'], 'passed')
        self.assertEqual(checks['rhwpLengthChangingRender']['page'], 0)

    def test_length_changing_checks_report_missing_parser(self):
        document = Document(make_hwp())
        with tempfile.TemporaryDirectory(dir=ROOT / 'work') as directory:
            with patch.object(corpus, 'probe', return_value={'available': False, 'reason': 'Node.js unavailable.'}):
                checks = corpus.length_changing_checks(document, Path(directory), 'synthetic.hwp')
        self.assertEqual(checks['lengthChangingEdit']['status'], 'passed')
        self.assertEqual(checks['rhwpLengthChangingEdit']['status'], 'skipped')
        self.assertEqual(checks['rhwpLengthChangingRender']['status'], 'skipped')
        self.assertIn('Node.js', checks['rhwpLengthChangingRender']['reason'])

    def test_empty_body_edit_is_skipped_and_corpus_is_partial(self):
        streams = dict(CompoundFile(make_hwp(False)).streams)
        streams['BodyText/Section0'] = record(66, b'\0' * 22)
        payload = cfb_from_streams(streams)
        item = {
            'name': 'empty.hwp',
            'bytes': len(payload),
            'blobSha1': hashlib.sha1(f'blob {len(payload)}\0'.encode() + payload).hexdigest(),
        }
        with tempfile.TemporaryDirectory(dir=ROOT / 'work') as directory:
            root = Path(directory)
            source_directory = root / 'fixtures/public'
            source_directory.mkdir(parents=True)
            (source_directory / item['name']).write_bytes(payload)
            output_directory = root / 'outputs'
            output_directory.mkdir()
            with patch.object(corpus, 'ROOT', root), patch.object(corpus, 'probe_rhwp', return_value={'status': 'skipped', 'reason': 'Node.js unavailable in this case.'}):
                result = corpus.test_fixture(item, output_directory)
        self.assertEqual(result['status'], 'partial')
        self.assertEqual(result['checks']['noop']['status'], 'passed')
        self.assertEqual(result['checks']['edit']['status'], 'skipped')
        self.assertEqual(result['checks']['edit']['reason'], 'No printable body text.')
        self.assertEqual(result['checks']['rhwpNoop']['status'], 'skipped')
        self.assertIn('rhwpProbe', result['checks'])
        self.assertEqual(result['checks']['anchorAgreement']['status'], 'skipped')
        self.assertEqual(result['checks']['anchorAgreement']['checkedBodyNodes'], 0)


class GeneratedFixtureTests(unittest.TestCase):
    def assert_rhwp_renders(self, payload, expected_text):
        result = probe_rhwp(payload)
        if result['status'] == 'skipped':
            self.skipTest(result['reason'])
        self.assertEqual(result['status'], 'passed', result.get('reason'))
        self.assertGreaterEqual(result['pageCount'], 1)
        self.assertEqual(result['renderedPage'], 0)
        self.assertIn(expected_text, result['text'])

    def assert_fixture_and_example(self, fixture_name, example_name, authored_payload):
        payload = (ROOT / 'fixtures/generated' / fixture_name).read_bytes()
        self.assertEqual(payload, authored_payload, 'Regenerate the checked-in fixture.')
        example = json.loads((ROOT / 'examples' / example_name).read_text())
        document = Document(payload)
        document.patch(example)
        saved = document.serialize(True)
        operation = example['operations'][0]
        self.assertEqual(Document(saved).by_id[operation['nodeId']].text, operation['text'])
        self.assert_rhwp_renders(payload, operation['expectedText'])
        self.assert_rhwp_renders(saved, operation['text'])

    def test_authored_hwp_opens_and_renders_in_rhwp(self):
        self.assert_fixture_and_example('legacy-review.hwp', 'legacy.patch.json', make_hwp())

    def test_authored_hwpx_opens_and_renders_in_rhwp(self):
        self.assert_fixture_and_example('review-form.hwpx', 'review.patch.json', make_hwpx())
