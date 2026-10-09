"""Memory previews retain native bytes; these fixtures do not certify Hancom."""
import base64
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from fixtures import make_hwp, make_hwpx
from hwp_workbench.document import Document, sha
from hwp_workbench.errors import WorkbenchError
from hwp_workbench.service import invoke, CAPABILITIES


class PreviewTests(unittest.TestCase):
    def test_native_preview_verified_without_any_file_write(self):
        for content in (make_hwp(), make_hwpx()):
            with tempfile.TemporaryDirectory() as folder:
                doc = Document(content)
                source = Path(folder) / ('source.' + doc.format)
                source.write_bytes(content)
                node = next(node for node in doc.nodes if node.editable)
                change = {'schemaVersion': 1, 'baseSha256': doc.hash, 'operations': [
                    {'op': 'set_text', 'nodeId': node.id, 'expectedText': node.text, 'text': node.text + ' 검토'}]}
                result = invoke(folder, 'preview', {'path': source.name, 'patch': change})
                preview = base64.b64decode(result['bytesBase64'], validate=True)
                self.assertEqual(result['sha256'], sha(preview))
                self.assertIn(node.text + ' 검토', [node.text for node in Document(preview).nodes])
                self.assertEqual(source.read_bytes(), content)
                self.assertEqual(list(Path(folder).iterdir()), [source])
                self.assertTrue(result['warnings'])
                change['baseSha256'] = '0' * 64
                with self.assertRaises(WorkbenchError) as caught:
                    invoke(folder, 'preview', {'path': source.name, 'patch': change})
                self.assertEqual(caught.exception.code, 'STALE_DOCUMENT')
                change['baseSha256'] = doc.hash
                change['operations'][0]['expectedText'] = 'wrong'
                with self.assertRaises(WorkbenchError) as caught:
                    invoke(folder, 'preview', {'path': source.name, 'patch': change})
                self.assertEqual(caught.exception.code, 'STALE_TEXT')

    def test_preview_size_cap_and_rpc_only_advertising(self):
        self.assertNotIn('preview', CAPABILITIES['operations'])
        with tempfile.TemporaryDirectory() as folder:
            content = make_hwpx()
            doc = Document(content)
            Path(folder, 'source.hwpx').write_bytes(content)
            node = doc.nodes[0]
            change = {'schemaVersion': 1, 'baseSha256': doc.hash, 'operations': [
                {'op': 'set_text', 'nodeId': node.id, 'expectedText': node.text, 'text': '검토'}]}
            with patch('hwp_workbench.service.MAX_PREVIEW_BYTES', 1):
                with self.assertRaises(WorkbenchError) as caught:
                    invoke(folder, 'preview', {'path': 'source.hwpx', 'patch': change})
                self.assertEqual(caught.exception.code, 'LIMIT')
