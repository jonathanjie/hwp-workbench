"""The independent parser must use bounded byte-only subprocess requests."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import unittest
from unittest.mock import patch

from fixtures import make_hwpx
from hwp_workbench.errors import WorkbenchError

ROOT = Path(__file__).resolve().parents[1]


class BridgeTests(unittest.TestCase):
    def run_frame(self, frame):
        node = shutil.which('node')
        if node is None:
            self.skipTest('Node.js is unavailable.')
        return subprocess.run([node, str(ROOT / 'engine/bridge/rhwp-bridge.mjs')],
                              input=frame, capture_output=True, timeout=30)

    def test_truncated_frame_is_json_error(self):
        result = self.run_frame(b'\x04\x00')
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stdout)['error']['code'], 'BAD_REQUEST')

    def test_path_argument_is_rejected(self):
        request = json.dumps({'op': 'probe', 'path': '/etc/passwd'}).encode()
        result = self.run_frame(struct.pack('<I', len(request)) + request + make_hwpx())
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stdout)['error']['code'], 'BAD_REQUEST')

    def test_frame_retains_binary_document_and_unicode(self):
        if not (ROOT / 'node_modules/@rhwp/core/rhwp_bg.wasm').is_file():
            self.skipTest('@rhwp/core or its WASM file is unavailable.')
        request = b'{"op":"probe"}'
        result = self.run_frame(struct.pack('<I', len(request)) + request + make_hwpx())
        self.assertEqual(result.returncode, 0, result.stderr)
        observed = json.loads(result.stdout)
        self.assertEqual(observed['version'], '0.8.7')
        self.assertEqual(observed['sections'][0]['paragraphs'][0]['text'], '내부 업무 검토서')


class WrapperTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.find_spec('hwp_workbench.independent')
        self.assertIsNotNone(spec, 'The bounded Python bridge wrapper is missing.')
        from hwp_workbench import independent
        self.bridge = independent

    def test_missing_node_is_unavailable(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(self.bridge.shutil, 'which', return_value=None):
            self.assertEqual(self.bridge.probe(make_hwpx())['available'], False)

    def test_missing_package_is_unavailable(self):
        with patch.object(self.bridge.shutil, 'which', return_value=sys.executable), \
                patch.object(self.bridge, 'PACKAGE', ROOT / 'work/absent-rhwp-package'):
            result = self.bridge.probe(make_hwpx())
        self.assertFalse(result['available'])
        self.assertIn('@rhwp/core', result['reason'])

    def test_output_cap_stops_child(self):
        with patch.object(self.bridge, 'MAX_OUTPUT', 1024):
            with self.assertRaises(WorkbenchError) as error:
                self.bridge._run_bridge([sys.executable, '-c', 'import sys; sys.stdout.buffer.write(b"x" * 65536)'], b'')
        self.assertEqual(error.exception.code, 'INDEPENDENT_LIMIT')

    def test_timeout_stops_child(self):
        with patch.object(self.bridge, 'TIMEOUT', 0.05):
            with self.assertRaises(WorkbenchError) as error:
                self.bridge._run_bridge([sys.executable, '-c', 'import time; time.sleep(10)'], b'')
        self.assertEqual(error.exception.code, 'INDEPENDENT_TIMEOUT')

    def test_bridge_errors_are_workbench_errors(self):
        available = self.bridge.probe(make_hwpx())
        if available.get('available') is False:
            self.skipTest(available['reason'])
        with self.assertRaises(WorkbenchError):
            self.bridge.probe(b'not a document')


class ApplyCheckTests(unittest.TestCase):
    def apply_with_probe(self, observed=None, error=None):
        import tempfile
        from hwp_workbench.service import invoke
        from hwp_workbench.document import Document
        from hwp_workbench import independent
        payload = make_hwpx()
        document = Document(payload)
        node = document.nodes[0]
        with tempfile.TemporaryDirectory(dir=ROOT / 'work') as directory:
            source = Path(directory) / 'source.hwpx'
            source.write_bytes(payload)
            with patch.object(independent, 'probe', return_value=observed, side_effect=error):
                result = invoke(directory, 'apply', {
                    'path': source.name, 'output': 'saved.hwpx', 'acknowledgeRisks': True,
                    'patch': {'schemaVersion': 1, 'baseSha256': document.hash, 'operations': [
                        {'op': 'set_text', 'nodeId': node.id, 'expectedText': node.text, 'text': 'Changed 😀'}]},
                })
            self.assertEqual(Document((Path(directory) / 'saved.hwpx').read_bytes()).nodes[0].text, 'Changed 😀')
            self.assertEqual(source.read_bytes(), payload)
        self.assertIn('independentCheck', result)
        return result['independentCheck'], node.id

    def test_apply_reports_confirmed_text(self):
        observed = {'engine': 'rhwp', 'version': '0.8.7', 'pageCount': 1,
                    'sections': [{'index': 0, 'paragraphs': [{'index': 0, 'text': 'Changed 😀'}]}]}
        result, _ = self.apply_with_probe(observed)
        self.assertEqual(result, {'engine': 'rhwp 0.8.7', 'opened': True, 'pageCount': 1,
                                  'editedTextsFound': True, 'missing': []})

    def test_apply_reports_missing_text(self):
        observed = {'engine': 'rhwp', 'version': '0.8.7', 'pageCount': 1, 'sections': []}
        result, node_id = self.apply_with_probe(observed)
        self.assertFalse(result['editedTextsFound'])
        self.assertEqual(result['missing'], [node_id])

    def test_apply_reports_unavailable_parser(self):
        observed = {'available': False, 'reason': 'Node.js is unavailable.'}
        result, _ = self.apply_with_probe(observed)
        self.assertEqual(result, observed)

    def test_independent_failure_does_not_block_verified_save(self):
        result, _ = self.apply_with_probe(error=WorkbenchError('RHWP_FAILED', 'Cannot open.'))
        self.assertFalse(result['opened'])
        self.assertEqual(result['error']['code'], 'RHWP_FAILED')


class BridgeOperationTests(unittest.TestCase):
    def setUp(self):
        from hwp_workbench import independent
        self.bridge = independent
        observed = independent.probe(make_hwpx())
        if observed.get('available') is False:
            self.skipTest(observed['reason'])

    def test_render_returns_requested_svg(self):
        result = self.bridge.invoke(make_hwpx(), 'render', page=0)
        self.assertEqual(result['page'], 0)
        self.assertIn('<svg', result['svg'])

    def test_out_of_range_page_fails(self):
        with self.assertRaises(WorkbenchError) as error:
            self.bridge.invoke(make_hwpx(), 'render', page=99999)
        self.assertEqual(error.exception.code, 'BAD_REQUEST')

    def test_exports_have_loss_report_and_native_bytes(self):
        import base64
        from hwp_workbench.document import Document
        for format in ('hwp', 'hwpx'):
            with self.subTest(format=format):
                result = self.bridge.invoke(make_hwpx(), 'export', format=format)
                self.assertEqual(result['format'], format)
                self.assertIsInstance(result['lossReport'], dict)
                exported = base64.b64decode(result['bytesBase64'], validate=True)
                self.assertEqual(Document(exported).format, format)

    def test_declared_json_length_is_bounded(self):
        node = shutil.which(os.environ.get('HWP_WORKBENCH_NODE') or 'node')
        with self.assertRaises(WorkbenchError) as error:
            self.bridge._run_bridge([node, str(self.bridge.BRIDGE)], struct.pack('<I', 4097))
        self.assertEqual(error.exception.code, 'BAD_REQUEST')
