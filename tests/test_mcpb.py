"""Bundle contents and startup must be independent of checkout scratch state."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile

from hwp_workbench.mcp import TOOLS

PROJECT = Path(__file__).resolve().parents[1]


class BundleTests(unittest.TestCase):
    def test_build_archive_manifest_no_overwrite_and_packaged_startup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'hwp-workbench.mcpb'
            command = [sys.executable, str(PROJECT / 'scripts' / 'build-mcpb.py'), '--output', str(output)]
            result = subprocess.run(command, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            with zipfile.ZipFile(output) as bundle:
                members = bundle.namelist()
                expected = {'manifest.json', 'launch.py'} | {
                    path.relative_to(PROJECT).as_posix()
                    for path in (PROJECT / 'engine' / 'hwp_workbench').rglob('*.py')
                    if '__pycache__' not in path.parts
                }
                if (PROJECT / 'mcpb' / 'icon.png').is_file():
                    expected.add('icon.png')
                self.assertEqual(set(members), expected)
                self.assertEqual(len(members), len(expected))
                manifest = json.loads(bundle.read('manifest.json'))
                self.assertEqual(manifest['tools'], [
                    {'name': tool['name'], 'description': tool['description']} for tool in TOOLS
                ])
                self.assertEqual(manifest['server']['type'], 'python')
                self.assertEqual(manifest['server']['entry_point'], 'launch.py')
                # The launcher runs on older system Python and hands over to Python 3.11+.
                self.assertEqual(manifest['server']['mcp_config'], {
                    'command': 'python3',
                    'args': ['${__dirname}/launch.py', 'mcp', '--root', '${user_config.workspace_directory}'],
                })
                self.assertEqual(manifest['compatibility'], {
                    'platforms': ['darwin'], 'runtimes': {'python': '>=3.8'},
                })
                workspace = manifest['user_config']['workspace_directory']
                self.assertEqual(workspace['type'], 'directory')
                self.assertTrue(workspace['required'])
                self.assertNotIn('default', workspace)
                self.assertEqual(manifest['icon'], 'icon.png')
                extracted = root / 'installed'
                bundle.extractall(extracted)
            original = output.read_bytes()
            result = subprocess.run(command, capture_output=True, text=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('exist', result.stderr.lower())
            self.assertEqual(output.read_bytes(), original)
            documents = root / 'documents'
            documents.mkdir()
            messages = [
                {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize'},
                {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
                {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'},
            ]
            config = manifest['server']['mcp_config']
            arguments = [argument.replace('${user_config.workspace_directory}', str(documents))
                         .replace('${__dirname}', str(extracted))
                         for argument in config['args']]
            environment = dict(os.environ)
            result = subprocess.run([sys.executable, *arguments], cwd=documents, env=environment,
                                    input='\n'.join(map(json.dumps, messages)) + '\n',
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            replies = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual(replies[-1]['result']['tools'], TOOLS)

    def test_builder_excludes_non_source_files_and_includes_optional_icon(self):
        import runpy

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / 'engine' / 'hwp_workbench'
            package.mkdir(parents=True)
            (package / '__init__.py').write_text('')
            (package / 'user.hwpx').write_bytes(b'private document')
            (package / 'credentials.json').write_text('private credentials')
            (package / '__pycache__').mkdir()
            (package / '__pycache__' / 'cached.pyc').write_bytes(b'cache')
            (root / 'mcpb').mkdir()
            shutil.copyfile(PROJECT / 'mcpb' / 'manifest.json', root / 'mcpb' / 'manifest.json')
            shutil.copyfile(PROJECT / 'mcpb' / 'launch.py', root / 'mcpb' / 'launch.py')
            (root / 'mcpb' / 'icon.png').write_bytes(b'test icon')
            builder = runpy.run_path(str(PROJECT / 'scripts' / 'build-mcpb.py'))
            output = root / 'bundle.mcpb'
            builder['build'](output, project=root)
            with zipfile.ZipFile(output) as bundle:
                self.assertEqual(set(bundle.namelist()), {
                    'manifest.json', 'launch.py', 'engine/hwp_workbench/__init__.py', 'icon.png',
                })
