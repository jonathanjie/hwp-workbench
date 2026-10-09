import tempfile
import unittest
from pathlib import Path
from hwp_workbench.service import invoke
from hwp_workbench.errors import WorkbenchError

ROOT = Path(__file__).resolve().parents[1]

class FormatTests(unittest.TestCase):
    def test_read_only_format_and_unreadable_fields(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / 'form.hwpx'
            source.write_bytes((ROOT / 'fixtures/generated/review-form.hwpx').read_bytes())
            original = source.read_bytes()
            opened = invoke(folder, 'inspect', {'path': source.name})
            result = invoke(folder, 'format', {'path': source.name, 'nodeId': opened['nodes'][0]['id']})
            self.assertEqual(result['character']['size'], 22)
            self.assertEqual(result['character']['font'], 'Apple SD Gothic Neo')
            self.assertEqual(result['character']['widthRatio'], 100)
            self.assertEqual(source.read_bytes(), original)
            with self.assertRaises(WorkbenchError):
                invoke(folder, 'format', {'path': source.name, 'nodeId': 'missing'})
            source = Path(folder) / 'legacy.hwp'
            source.write_bytes((ROOT / 'fixtures/generated/legacy-review.hwp').read_bytes())
            unknown = invoke(folder, 'format', {'path': source.name})
            self.assertEqual(unknown['character'], {})
