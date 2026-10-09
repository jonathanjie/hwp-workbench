"""Agent setup and reviewed-preview workflows use isolated document folders."""
import contextlib
import copy
import io
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from fixtures import make_hwpx
from hwp_workbench.cli import main
from hwp_workbench.document import Document
from hwp_workbench.errors import WorkbenchError
from hwp_workbench.mcp import serve
from hwp_workbench.service import invoke


class AgentReachTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.source = make_hwpx()
        (self.root / 'form.hwpx').write_bytes(self.source)

    def cli(self, arguments, request=None):
        stdout = io.StringIO()
        stderr = io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            with patch('sys.stdin', io.StringIO(request or '')):
                status = main(arguments)
        return status, stdout.getvalue(), stderr.getvalue()

    def mcp(self, method, args):
        messages = [
            {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize'},
            {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
            {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call',
             'params': {'name': 'hwp_' + method, 'arguments': args}},
        ]
        stdout = io.StringIO()
        serve(self.root, io.StringIO('\n'.join(map(json.dumps, messages)) + '\n'), stdout)
        return json.loads(stdout.getvalue().splitlines()[-1])

    def test_preview_envelopes_plan_and_apply_in_cli_mcp_and_rpc(self):
        previews = [
            invoke(self.root, 'replace', {'path': 'form.hwpx', 'find': '검토', 'replaceWith': '확정'}),
            invoke(self.root, 'fill', {'path': 'form.hwpx', 'fields': {'기관명': 'New organisation'}}),
            invoke(self.root, 'set', {'path': 'form.hwpx', 'nodeId': 's0:t0', 'text': 'Reviewed text'}),
        ]
        for index, preview in enumerate(previews):
            (self.root / 'preview.json').write_text(json.dumps(preview))
            expected = invoke(self.root, 'plan', {'path': 'form.hwpx', 'patch': preview['patch']})
            for transport in ['cli', 'mcp', 'rpc']:
                for method in ['plan', 'apply']:
                    with self.subTest(preview=index, transport=transport, method=method):
                        args = {'path': 'form.hwpx', 'patch': preview}
                        flags = []
                        output = f'reviewed-{index}-{transport}.hwpx'
                        if method == 'apply':
                            args.update(output=output, acknowledgeRisks=True)
                            flags = ['-o', output, '--acknowledge-risks']
                        if transport == 'mcp':
                            reply = self.mcp(method, args)
                            self.assertNotIn('error', reply, reply)
                            self.assertFalse(reply['result']['isError'], reply)
                            result = reply['result']['structuredContent']
                        else:
                            command = [method, 'form.hwpx', '--patch', 'preview.json', *flags]
                            request = None
                            if transport == 'rpc':
                                command = ['rpc']
                                request = json.dumps({'method': method, 'args': args})
                            status, stdout, stderr = self.cli([*command, '--root', str(self.root)], request)
                            self.assertEqual(status, 0, stderr)
                            self.assertEqual(stderr, '')
                            result = json.loads(stdout)
                        self.assertEqual(result['changes'], expected['changes'])
                        if method == 'apply':
                            self.assertTrue(result['sourceUntouched'])
                            saved = Document((self.root / output).read_bytes())
                            for operation in preview['patch']['operations']:
                                self.assertEqual(saved.by_id[operation['nodeId']].text, operation['text'])
        self.assertEqual((self.root / 'form.hwpx').read_bytes(), self.source)

    def test_envelope_keeps_complete_patch_validation_and_no_overwrite(self):
        preview = invoke(self.root, 'set', {'path': 'form.hwpx', 'nodeId': 's0:t0', 'text': 'Reviewed'})
        invalid = []
        for field, value, code in [('baseSha256', '0' * 64, 'STALE_DOCUMENT'),
                                   ('expectedText', 'wrong', 'STALE_TEXT')]:
            envelope = copy.deepcopy(preview)
            target = envelope['patch'] if field == 'baseSha256' else envelope['patch']['operations'][0]
            target[field] = value
            invalid.append((envelope, code))
        envelope = copy.deepcopy(preview)
        envelope['patch']['operations'].append(copy.deepcopy(envelope['patch']['operations'][0]))
        invalid.append((envelope, 'BAD_PATCH'))
        invalid.append(({'patch': {'schemaVersion': 1, 'baseSha256': preview['baseSha256'],
                                   'operations': []}}, 'BAD_PATCH'))
        invalid.append(({'patch': preview['patch'], 'schemaVersion': 2}, 'BAD_PATCH'))
        for envelope, code in invalid:
            with self.subTest(code=code), self.assertRaises(WorkbenchError) as caught:
                invoke(self.root, 'apply', {'path': 'form.hwpx', 'patch': envelope,
                                          'output': 'invalid.hwpx', 'acknowledgeRisks': True})
            self.assertEqual(caught.exception.code, code)
            self.assertFalse((self.root / 'invalid.hwpx').exists())
        (self.root / 'existing.hwpx').write_bytes(b'keep this output')
        with self.assertRaises(WorkbenchError) as caught:
            invoke(self.root, 'apply', {'path': 'form.hwpx', 'patch': preview,
                                      'output': 'existing.hwpx', 'acknowledgeRisks': True})
        self.assertEqual(caught.exception.code, 'OUTPUT_EXISTS')
        self.assertEqual((self.root / 'existing.hwpx').read_bytes(), b'keep this output')

    def test_mcp_config_prints_exact_setup_for_each_client(self):
        project = Path(__file__).resolve().parents[1]
        documents = self.root / "Document folder's name"
        documents.mkdir()
        launcher = str(project / 'scripts' / 'hwp-workbench')
        cases = {
            'claude-code': ['claude', 'mcp', 'add', 'hwp-workbench', '--scope', 'user', '--',
                            launcher, 'mcp', '--root', str(documents)],
            'codex': ['codex', 'mcp', 'add', 'hwp-workbench', '--',
                      launcher, 'mcp', '--root', str(documents)],
        }
        with patch('subprocess.run') as run, patch('subprocess.Popen') as launch:
            for client, command in cases.items():
                with self.subTest(client=client):
                    status, stdout, stderr = self.cli(['mcp-config', '--client', client,
                                                     '--root', str(documents)])
                    self.assertEqual(status, 0, stderr)
                    self.assertEqual(stdout, shlex.join(command) + '\n')
                    self.assertEqual(stderr, '')
            status, stdout, stderr = self.cli(['mcp-config', '--client', 'json', '--root', str(documents)])
            self.assertEqual(status, 0, stderr)
            self.assertEqual(json.loads(stdout), {'mcpServers': {'hwp-workbench': {
                'command': launcher, 'args': ['mcp', '--root', str(documents)]}}})
            status, stdout, stderr = self.cli(['mcp-config', '--client', 'claude-desktop',
                                              '--root', str(documents)])
            self.assertEqual(status, 0, stderr)
            self.assertEqual(stdout.strip(), str(project / 'dist' / 'hwp-workbench.mcpb'))
            run.assert_not_called()
            launch.assert_not_called()
        self.assertEqual(list(documents.iterdir()), [])

    def test_mcp_config_refuses_missing_file_home_and_filesystem_roots(self):
        alias = self.root / 'home-alias'
        alias.symlink_to(Path.home(), target_is_directory=True)
        for root in [str(self.root / 'missing'), str(self.root / 'form.hwpx'),
                     str(Path.home()), '/', str(alias)]:
            with self.subTest(root=root):
                status, stdout, stderr = self.cli(['mcp-config', '--client', 'json', '--root', root])
                self.assertEqual(status, 2)
                self.assertEqual(stdout, '')
                error = json.loads(stderr)['error']
                self.assertEqual(error['code'], 'BAD_ROOT')
                self.assertIn('document', error['hint'].lower())

    def desktop_environment(self):
        from hwp_workbench import desktop_open

        checkout = self.root / 'checkout'
        checkout.mkdir()
        (checkout / 'package.json').write_text('{"main":"desktop/main.cjs"}')
        (checkout / 'desktop').mkdir()
        (checkout / 'desktop' / 'main.cjs').write_text('// test source checkout')
        applications = self.root / 'Applications'
        user_applications = self.root / 'User Applications'
        applications.mkdir()
        user_applications.mkdir()
        for name, value in [('PROJECT', checkout), ('APPLICATION_DIRECTORIES', (applications, user_applications))]:
            self.enterContext(patch.object(desktop_open, name, value))
        self.enterContext(patch.object(desktop_open.sys, 'platform', 'darwin'))
        return checkout, applications, user_applications

    def test_open_prefers_packaged_app_in_both_application_folders(self):
        checkout, applications, user_applications = self.desktop_environment()
        with patch('subprocess.run') as run, patch('subprocess.Popen') as launch:
            for folder in [applications, user_applications]:
                with self.subTest(folder=folder):
                    app = folder / 'HWP Workbench.app'
                    app.mkdir()
                    status, stdout, stderr = self.cli(['open', 'form.hwpx', '--root', str(self.root)])
                    self.assertEqual(status, 0, stderr)
                    self.assertEqual(json.loads(stdout), {'opened': True, 'via': 'app'})
                    run.assert_called_with(['open', '-a', 'HWP Workbench', str(self.root / 'form.hwpx')],
                                           check=True, capture_output=True, text=True, timeout=10)
                    launch.assert_not_called()
                    app.rmdir()

    def test_open_source_electron_is_detached_and_uses_absolute_paths(self):
        checkout, _, _ = self.desktop_environment()
        electron = checkout / 'node_modules' / '.bin' / 'electron'
        electron.parent.mkdir(parents=True)
        electron.write_text('#!/bin/sh\n')
        electron.chmod(0o700)
        with patch('subprocess.run') as run, patch('subprocess.Popen') as launch:
            status, stdout, stderr = self.cli(['open', str(self.root / 'form.hwpx'), '--root', str(self.root)])
            self.assertEqual(status, 0, stderr)
            self.assertEqual(json.loads(stdout), {'opened': True, 'via': 'electron'})
            launch.assert_called_once_with([str(electron), str(checkout), '--open', str(self.root / 'form.hwpx')],
                                           cwd=checkout, start_new_session=True, stdin=subprocess.DEVNULL,
                                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            run.assert_not_called()

    def test_open_rejects_unsafe_paths_before_launch(self):
        self.desktop_environment()
        (self.root / 'wrong.pdf').write_bytes(b'document')
        (self.root / 'folder.hwpx').mkdir()
        (self.root / 'linked.hwpx').symlink_to(self.root / 'form.hwpx')
        (self.root / 'linked-folder').symlink_to(self.root, target_is_directory=True)
        with patch('subprocess.run') as run, patch('subprocess.Popen') as launch:
            for path in ['missing.hwpx', 'wrong.pdf', 'folder.hwpx', 'linked.hwpx',
                         'linked-folder/form.hwpx', '../outside.hwpx', '/outside.hwpx']:
                with self.subTest(path=path):
                    status, stdout, stderr = self.cli(['open', path, '--root', str(self.root)])
                    self.assertEqual(status, 2)
                    self.assertEqual(stdout, '')
                    self.assertTrue(json.loads(stderr)['error']['hint'])
            run.assert_not_called()
            launch.assert_not_called()

    def test_open_reports_missing_app_and_launch_failure_with_hints(self):
        from hwp_workbench import desktop_open

        self.desktop_environment()
        status, stdout, stderr = self.cli(['open', 'form.hwpx', '--root', str(self.root)])
        self.assertEqual(status, 2)
        self.assertEqual(json.loads(stderr)['error']['code'], 'APP_UNAVAILABLE')
        self.assertIn('Electron', json.loads(stderr)['error']['hint'])
        with patch.object(desktop_open, '_packaged_app', return_value=True):
            with patch('subprocess.run', side_effect=subprocess.CalledProcessError(1, 'open', stderr='launch failed')):
                status, stdout, stderr = self.cli(['open', 'form.hwpx', '--root', str(self.root)])
                self.assertEqual(status, 2)
                self.assertEqual(stdout, '')
                self.assertEqual(json.loads(stderr)['error']['code'], 'OPEN_FAILED')
                self.assertTrue(json.loads(stderr)['error']['hint'])
