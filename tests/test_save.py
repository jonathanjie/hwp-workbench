"""Save-in-place safety checks use authored bytes and system temporary folders."""
import hashlib
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

from hwp_workbench.errors import WorkbenchError
from hwp_workbench.workspace import Workspace


def digest(data):
    return hashlib.sha256(data).hexdigest()


class WorkspaceFixture:
    format = 'hwpx'

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.path = self.root / 'form.hwpx'
        self.original = b'original document bytes'
        self.path.write_bytes(self.original)
        self.path.chmod(0o640)
        self.workspace = Workspace(self.root)

    def expect_code(self, code, operation):
        with self.assertRaises(WorkbenchError) as caught:
            operation()
        self.assertEqual(caught.exception.code, code)

    def replace(self):
        return self.workspace.replace_existing('form.hwpx', b'edited bytes', digest(self.original))

    def assert_original(self):
        self.assertEqual(self.path.read_bytes(), self.original)
        self.assertEqual(sorted(item.name for item in self.root.iterdir()), ['form.hwpx'])


class WorkspaceReplaceTests(WorkspaceFixture, unittest.TestCase):
    def test_replace_reads_back_hash_and_keeps_mode(self):
        self.assertTrue(callable(getattr(self.workspace, 'replace_existing', None)),
                        'Workspace.replace_existing is required')
        result = self.replace()
        self.assertEqual(result, digest(self.path.read_bytes()))
        self.assertEqual(self.path.read_bytes(), b'edited bytes')
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o640)
        self.assertEqual(list(self.root.glob('*.tmp')), [])

    def test_changed_source_is_refused(self):
        self.path.write_bytes(b'external edit')
        self.expect_code('SOURCE_CHANGED', self.replace)
        self.assertEqual(self.path.read_bytes(), b'external edit')
        self.assertEqual(list(self.root.glob('*.tmp')), [])

    def test_symlink_is_refused(self):
        target = self.root / ('target.' + self.format)
        self.path.rename(target)
        self.path.symlink_to(target)
        self.expect_code('UNSAFE_PATH', self.replace)
        self.assertEqual(target.read_bytes(), self.original)
        self.assertTrue(self.path.is_symlink())

    def test_read_only_is_refused(self):
        self.path.chmod(0o400)
        self.expect_code('READ_ONLY_DOCUMENT', self.replace)
        self.assert_original()

    def test_nonregular_is_refused_without_blocking(self):
        self.path.unlink()
        os.mkfifo(self.path)
        self.expect_code('UNSAFE_PATH', self.replace)

    def test_rename_failure_keeps_original_and_removes_temporary_files(self):
        with patch('hwp_workbench.workspace.os.replace', side_effect=OSError('injected rename failure')):
            self.expect_code('WRITE_FAILED', self.replace)
        self.assert_original()

    def test_directory_sync_failure_after_rename_restores_original(self):
        real_sync = os.fsync
        failed = False

        def sync(descriptor):
            nonlocal failed
            if stat.S_ISDIR(os.fstat(descriptor).st_mode) and not failed:
                failed = True
                raise OSError('injected directory sync failure')
            return real_sync(descriptor)

        with patch('hwp_workbench.workspace.os.fsync', side_effect=sync):
            self.expect_code('WRITE_FAILED', self.replace)
        self.assert_original()

    def test_temporary_collision_does_not_remove_existing_file(self):
        collision = self.root / ('.hwp-workbench-' + 'a' * 24 + '.tmp')
        collision.write_bytes(b'belongs to another operation')
        with patch('hwp_workbench.workspace.secrets.token_hex', return_value='a' * 24):
            self.expect_code('WRITE_FAILED', self.replace)
        self.assertEqual(self.path.read_bytes(), self.original)
        self.assertEqual(collision.read_bytes(), b'belongs to another operation')


class PreviousVersionTests(WorkspaceFixture, unittest.TestCase):
    def test_shared_backup_folder_isolates_same_named_sources(self):
        directory = self.root / 'backups'
        directory.mkdir()
        legacy = directory / 'form.20260101T000000Z-abcdef.hwpx'
        legacy.write_bytes(b'legacy previous version')
        saved = {}
        for source in ('a/form.hwpx', 'b/form.hwpx'):
            path = self.root / source
            path.parent.mkdir()
            path.write_bytes(source.encode())
            saved[source] = []
            for revision in range(12):
                payload = f'{source} revision {revision}'.encode()
                name = self.workspace.backup_existing(source, payload, 'backups')
                saved[source].append((self.root / name, payload))
        for source, backups in saved.items():
            kept = [(path, payload) for path, payload in backups if path.exists()]
            self.assertEqual(len(kept), 10, source)
            for path, payload in kept:
                self.assertEqual(path.read_bytes(), payload)
        self.assertEqual(legacy.read_bytes(), b'legacy previous version')

    def backup(self, directory=None):
        return self.workspace.backup_existing('form.hwpx', self.original, directory)

    def test_backup_bytes_name_and_private_modes(self):
        self.assertTrue(callable(getattr(self.workspace, 'backup_existing', None)),
                        'Workspace.backup_existing is required')
        result = self.backup()
        backup = self.root / result
        self.assertRegex(backup.name, r'^form\.[0-9a-f]{8}\.[0-9]{8}T[0-9]{6}Z-[0-9a-f]{6}\.hwpx$')
        self.assertEqual(backup.read_bytes(), self.original)
        self.assertEqual(stat.S_IMODE(backup.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(backup.parent.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(backup.parent.parent.stat().st_mode), 0o700)
        self.assertEqual(self.path.read_bytes(), self.original)

    def test_retention_keeps_newest_ten_and_preserves_other_entries(self):
        directory = self.root / 'backups'
        directory.mkdir()
        identity = digest(os.fsencode(self.path.resolve()))[:8]
        old = []
        for index in range(12):
            name = f'form.{identity}.20260101T000000Z-{index:06x}.hwpx'
            entry = directory / name
            entry.write_bytes(str(index).encode())
            os.utime(entry, ns=(index + 1, index + 1))
            old.append(name)
        unrelated = ['form.other.20260101T000000Z-000000.hwpx', 'form.txt',
                     'form.20260101T000000Z-000000.hwp', 'form.20260101T000000Z-abcdefg.hwpx']
        for name in unrelated:
            (directory / name).write_bytes(b'keep')
        linked = directory / 'form.20260101T000000Z-ffffff.hwpx'
        linked.symlink_to(self.path)
        folder = directory / 'form.20260101T000000Z-eeeeee.hwpx'
        folder.mkdir()
        result = self.backup('backups')
        regular = [entry for entry in directory.iterdir()
                   if entry.is_file() and not entry.is_symlink() and entry.name not in unrelated]
        self.assertEqual(len(regular), 10)
        self.assertIn(self.root / result, regular)
        self.assertEqual(sorted(entry.name for entry in regular if entry.name in old), old[3:])
        for name in unrelated:
            self.assertEqual((directory / name).read_bytes(), b'keep')
        self.assertTrue(linked.is_symlink())
        self.assertTrue(folder.is_dir())

    def test_symlink_backup_directory_is_refused(self):
        (self.root / 'real-backups').mkdir()
        (self.root / 'backups').symlink_to(self.root / 'real-backups')
        self.expect_code('UNSAFE_PATH', lambda: self.backup('backups'))
        self.assertEqual(list((self.root / 'real-backups').iterdir()), [])

    def test_backup_directory_stays_inside_workspace(self):
        self.expect_code('OUTSIDE_WORKSPACE', lambda: self.backup('../backups'))

    def test_backup_collision_preserves_previous_file(self):
        directory = self.root / 'backups'
        directory.mkdir()
        with patch('hwp_workbench.workspace.datetime') as clock:
            clock.now.return_value.strftime.return_value = '20260101T000000Z'
            self.enterContext(patch('hwp_workbench.workspace.secrets.token_hex', return_value='abcdef'))
            first = self.backup('backups')
            self.expect_code('OUTPUT_EXISTS', lambda: self.backup('backups'))
        self.assertEqual((self.root / first).read_bytes(), self.original)
        self.assertEqual(len(list(directory.iterdir())), 1)


class ServiceSaveFixture:
    format = 'hwpx'

    def setUp(self):
        from fixtures import make_hwp, make_hwpx
        from hwp_workbench.document import Document
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.path = self.root / ('form.' + self.format)
        self.original = make_hwp() if self.format == 'hwp' else make_hwpx()
        self.path.write_bytes(self.original)
        self.path.chmod(0o640)
        self.document = Document(self.original)
        node = self.document.nodes[0]
        self.new_text = 'Saved Korean text 한글'
        self.native_patch = {'schemaVersion': 1, 'baseSha256': self.document.hash,
                             'operations': [{'op': 'set_text', 'nodeId': node.id,
                                             'expectedText': node.text, 'text': self.new_text}]}
        self.arguments = {'path': self.path.name, 'patch': self.native_patch, 'acknowledgeRisks': True}
        self.independent = {'engine': 'test evidence', 'opened': True,
                            'editedTextsFound': True, 'missing': []}

    def save(self):
        from hwp_workbench.service import invoke
        return invoke(self.root, 'save', self.arguments)

    def expect_code(self, code, operation):
        with self.assertRaises(WorkbenchError) as caught:
            operation()
        self.assertEqual(caught.exception.code, code)

    def assert_original(self, backups=0):
        self.assertEqual(self.path.read_bytes(), self.original)
        self.assertEqual(list(self.root.rglob('*.tmp')), [])
        self.assertEqual(len(list(self.root.rglob('*.' + self.format))), backups + 1)


class ServiceSaveTests(ServiceSaveFixture, unittest.TestCase):
    def test_save_replaces_verified_native_bytes_and_keeps_previous_version(self):
        from hwp_workbench.document import Document
        from hwp_workbench.service import CAPABILITIES
        self.assertIn('save', CAPABILITIES['operations'])
        with patch('hwp_workbench.service.check_output', return_value=self.independent):
            result = self.save()
        self.assertTrue(result['saved'])
        self.assertEqual(result['path'], self.path.name)
        self.assertEqual(result['previousSha256'], digest(self.original))
        self.assertEqual(result['sha256'], digest(self.path.read_bytes()))
        self.assertEqual(Document(self.path.read_bytes()).nodes[0].text, self.new_text)
        self.assertEqual((self.root / result['backup']).read_bytes(), self.original)
        self.assertEqual(result['independentCheck'], self.independent)
        self.assertIn('LAYOUT_UNVERIFIED', [warning['code'] for warning in result['warnings']])
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o640)

    def test_verification_failure_before_backup_leaves_original(self):
        failure = WorkbenchError('VERIFY_FAILED', 'injected verification failure')
        with patch('hwp_workbench.document.Document.serialize', side_effect=failure):
            self.expect_code('VERIFY_FAILED', self.save)
        self.assert_original()

    def test_independent_reopen_or_missing_text_blocks_save(self):
        for result in [{'opened': False, 'error': {'code': 'FAILED'}},
                       {'opened': True, 'editedTextsFound': False, 'missing': ['node']}]:
            with self.subTest(result=result):
                with patch('hwp_workbench.service.check_output', return_value=result):
                    self.expect_code('VERIFY_FAILED', self.save)
                self.assert_original()

    def test_unavailable_independent_check_is_reported(self):
        unavailable = {'available': False, 'reason': 'Parser unavailable in this test'}
        with patch('hwp_workbench.service.check_output', return_value=unavailable):
            result = self.save()
        self.assertEqual(result['independentCheck'], unavailable)

    def test_backup_failure_leaves_original(self):
        with patch('hwp_workbench.service.check_output', return_value=self.independent):
            with patch.object(Workspace, 'write_new', side_effect=WorkbenchError('WRITE_FAILED', 'injected backup failure')):
                self.expect_code('WRITE_FAILED', self.save)
        self.assert_original()

    def test_rename_failure_after_backup_leaves_original_and_keeps_backup(self):
        with patch('hwp_workbench.service.check_output', return_value=self.independent):
            with patch('hwp_workbench.workspace.os.replace', side_effect=OSError('injected rename failure')):
                self.expect_code('WRITE_FAILED', self.save)
        self.assert_original(backups=1)
        backups = list((self.root / '.hwp-workbench/backups').iterdir())
        self.assertEqual(backups[0].read_bytes(), self.original)

    def test_file_changed_during_verification_is_refused(self):
        def independent_check(*args):
            self.path.write_bytes(b'external change')
            return self.independent

        with patch('hwp_workbench.service.check_output', side_effect=independent_check):
            self.expect_code('SOURCE_CHANGED', self.save)
        self.assertEqual(self.path.read_bytes(), b'external change')
        self.assertEqual(list(self.root.rglob('*.tmp')), [])
        self.assertFalse((self.root / '.hwp-workbench').exists())

    def test_changed_file_after_read_refuses_stale_patch(self):
        from hwp_workbench.document import Document
        newer = Document(self.original)
        newer.patch(self.native_patch)
        changed = newer.serialize(True)
        self.path.write_bytes(changed)
        self.expect_code('SOURCE_CHANGED', self.save)
        self.assertEqual(self.path.read_bytes(), changed)

    def test_hash_rechecked_after_backup(self):
        original_backup = Workspace.backup_existing

        def backup_then_change(workspace, *args):
            result = original_backup(workspace, *args)
            self.path.write_bytes(b'external edit after backup')
            return result

        with patch('hwp_workbench.service.check_output', return_value=self.independent):
            with patch.object(Workspace, 'backup_existing', backup_then_change):
                self.expect_code('SOURCE_CHANGED', self.save)
        self.assertEqual(self.path.read_bytes(), b'external edit after backup')
        self.assertEqual(list(self.root.rglob('*.tmp')), [])

    def test_wrong_native_extension_refused(self):
        extension = 'hwpx' if self.format == 'hwp' else 'hwp'
        wrong_path = self.root / ('form.' + extension)
        self.path.rename(wrong_path)
        self.arguments['path'] = wrong_path.name
        self.expect_code('OUTPUT_FORMAT', self.save)
        self.assertEqual(wrong_path.read_bytes(), self.original)

    def test_risk_acknowledgement_required(self):
        self.arguments['acknowledgeRisks'] = False
        self.expect_code('RISK_ACK_REQUIRED', self.save)
        self.assert_original()

    def test_incomplete_patch_and_wrong_expected_text_refused(self):
        operation = self.native_patch['operations'][0]
        operation.pop('text')
        self.expect_code('BAD_PATCH', self.save)
        self.assert_original()
        operation['text'] = self.new_text
        operation['expectedText'] = 'wrong text'
        self.expect_code('STALE_TEXT', self.save)
        self.assert_original()


class SaveCliTests(ServiceSaveFixture, unittest.TestCase):
    def run_cli(self, *arguments):
        import subprocess
        executable = Path(__file__).resolve().parents[1] / 'scripts/hwp-workbench'
        environment = os.environ.copy()
        environment['HWP_WORKBENCH_NODE'] = '__unavailable_for_cli_test__'
        return subprocess.run([str(executable), *arguments, '--root', str(self.root)],
                              capture_output=True, text=True, env=environment, timeout=30)

    def test_cli_save_accepts_preview_envelope_and_backup_directory(self):
        import json
        from hwp_workbench.document import Document
        patch_path = self.root / 'preview.json'
        patch_path.write_text(json.dumps({'patch': self.native_patch, 'changes': []}))
        result = self.run_cli('save', self.path.name, '--patch', 'preview.json',
                              '--acknowledge-risks', '--backup-dir', 'versions')
        self.assertEqual(result.returncode, 0, result.stderr)
        saved = json.loads(result.stdout)
        self.assertTrue(saved['saved'])
        self.assertEqual(Document(self.path.read_bytes()).nodes[0].text, self.new_text)
        self.assertEqual(Path(saved['backup']).parent, Path('versions'))
        self.assertEqual((self.root / saved['backup']).read_bytes(), self.original)
        self.assertFalse(saved['independentCheck']['available'])
        stale = self.run_cli('save', self.path.name, '--patch', 'preview.json', '--acknowledge-risks')
        self.assertEqual(stale.returncode, 2)
        self.assertEqual(json.loads(stale.stderr)['error']['code'], 'SOURCE_CHANGED')

    def test_cli_save_requires_risk_acknowledgement(self):
        import json
        (self.root / 'patch.json').write_text(json.dumps(self.native_patch))
        result = self.run_cli('save', self.path.name, '--patch', 'patch.json')
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stderr)['error']['code'], 'RISK_ACK_REQUIRED')
        self.assert_original()


class HwpServiceSaveTests(ServiceSaveTests):
    format = 'hwp'


class HwpSaveCliTests(SaveCliTests):
    format = 'hwp'


class SaveSafetyTests(ServiceSaveFixture, unittest.TestCase):
    def test_save_refuses_read_only_document(self):
        self.path.chmod(0o400)
        with patch('hwp_workbench.service.check_output', return_value=self.independent):
            self.expect_code('READ_ONLY_DOCUMENT', self.save)
        self.assert_original(backups=1)

    def test_save_refuses_symlink_document(self):
        target = self.root / ('target.' + self.format)
        self.path.rename(target)
        self.path.symlink_to(target)
        self.expect_code('READ_FAILED', self.save)
        self.assertEqual(target.read_bytes(), self.original)
        self.assertTrue(self.path.is_symlink())

    def test_save_refuses_symlink_backup_folder(self):
        (self.root / 'versions').mkdir()
        (self.root / '.hwp-workbench').symlink_to(self.root / 'versions')
        with patch('hwp_workbench.service.check_output', return_value=self.independent):
            self.expect_code('UNSAFE_PATH', self.save)
        self.assert_original()
        self.assertEqual(list((self.root / 'versions').iterdir()), [])

    def test_backup_verification_failure_after_backup_leaves_original(self):
        write_new = Workspace.write_new

        def corrupt_backup(workspace, path, data):
            return write_new(workspace, path, b'injected corrupted backup')

        with patch('hwp_workbench.service.check_output', return_value=self.independent):
            with patch.object(Workspace, 'write_new', corrupt_backup):
                self.expect_code('VERIFY_FAILED', self.save)
        self.assert_original(backups=1)

    def test_writer_cannot_change_native_format(self):
        from fixtures import make_hwp
        with patch('hwp_workbench.document.Document.serialize', return_value=make_hwp()):
            with patch('hwp_workbench.service.check_output', return_value=self.independent):
                self.expect_code('VERIFY_FAILED', self.save)
        self.assert_original()

    def test_empty_edit_remains_independently_unverified(self):
        from hwp_workbench.document import Document
        self.native_patch['operations'][0]['text'] = ''
        unverified = {'opened': True, 'editedTextsFound': False, 'missing': [], 'unverified': ['node']}
        with patch('hwp_workbench.service.check_output', return_value=unverified):
            result = self.save()
        self.assertEqual(Document(self.path.read_bytes()).nodes[0].text, '')
        self.assertEqual(result['independentCheck'], unverified)

    def test_apply_still_refuses_overwrite_and_does_not_gate_independent_check(self):
        from hwp_workbench.service import invoke
        arguments = dict(self.arguments, output=self.path.name)
        unavailable = {'opened': False, 'error': {'code': 'FAILED'}}
        with patch('hwp_workbench.service.check_output', return_value=unavailable):
            self.expect_code('OUTPUT_EXISTS', lambda: invoke(self.root, 'apply', arguments))
            self.assert_original()
            arguments['output'] = 'new.' + self.format
            result = invoke(self.root, 'apply', arguments)
        self.assertTrue(result['sourceUntouched'])
        self.assertEqual(result['independentCheck'], unavailable)
        self.assertEqual(self.path.read_bytes(), self.original)

    def test_save_is_not_exposed_to_mcp(self):
        from hwp_workbench.mcp import _tools
        from hwp_workbench.service import CAPABILITIES
        self.assertNotIn('hwp_save', [tool['name'] for tool in _tools()])
        self.assertEqual(CAPABILITIES['safety']['overwrite'], 'save operation only, verified, with backup')

    def test_real_independent_reader_reopens_saved_output(self):
        from hwp_workbench.independent import probe
        available = probe(self.original)
        if available.get('available') is False:
            self.skipTest(available['reason'])
        result = self.save()
        self.assertTrue(result['independentCheck']['opened'])
        self.assertTrue(result['independentCheck']['editedTextsFound'])


class HwpIndependentSaveTests(SaveSafetyTests):
    format = 'hwp'

    def test_writer_cannot_change_native_format(self):
        from fixtures import make_hwpx
        with patch('hwp_workbench.document.Document.serialize', return_value=make_hwpx()):
            with patch('hwp_workbench.service.check_output', return_value=self.independent):
                self.expect_code('VERIFY_FAILED', self.save)
        self.assert_original()


class ReplacementFailureTests(WorkspaceFixture, unittest.TestCase):
    def test_concurrent_writer_after_publish_keeps_revision_and_recovery(self):
        for change in ('write', 'replace', 'same-bytes-replace'):
            with self.subTest(change=change):
                self.path.write_bytes(self.original)
                real_replace = os.replace
                published = False

                def publish_then_change(*args, **kwargs):
                    nonlocal published
                    result = real_replace(*args, **kwargs)
                    if not published:
                        published = True
                        external = b'edited bytes' if change == 'same-bytes-replace' else b'external edit'
                        if change == 'write':
                            self.path.write_bytes(external)
                        else:
                            writer = self.root / 'external.tmp'
                            writer.write_bytes(external)
                            real_replace(writer, self.path)
                    return result

                with patch('hwp_workbench.workspace.os.replace', side_effect=publish_then_change):
                    with self.assertRaises(WorkbenchError) as caught:
                        self.replace()
                self.assertEqual(caught.exception.code, 'SOURCE_CHANGED')
                expected = b'edited bytes' if change == 'same-bytes-replace' else b'external edit'
                self.assertEqual(self.path.read_bytes(), expected)
                recovery_files = list((self.root / '.hwp-workbench/backups').glob('*recovery*'))
                self.assertTrue(recovery_files)
                recovery = next(entry for entry in recovery_files
                                if str(entry.relative_to(self.root)) in str(caught.exception))
                self.assertEqual(recovery.read_bytes(), self.original)
                self.assertIn(str(recovery.relative_to(self.root)), str(caught.exception))
                self.assertEqual(list(self.root.glob('*.tmp')), [])

    def test_final_source_check_catches_change_during_temporary_write(self):
        private_file = self.workspace._private_file
        calls = 0

        def write_then_change(*args):
            nonlocal calls
            name = private_file(*args)
            calls += 1
            if calls == 2:
                self.path.write_bytes(b'external edit during temporary write')
            return name

        with patch.object(self.workspace, '_private_file', side_effect=write_then_change):
            self.expect_code('SOURCE_CHANGED', self.replace)
        self.assertEqual(self.path.read_bytes(), b'external edit during temporary write')
        self.assertEqual(list(self.root.glob('*.tmp')), [])

    def test_readback_failure_restores_source_and_mode(self):
        read_source = self.workspace._replacement_source
        calls = 0

        def fail_readback(*args):
            nonlocal calls
            calls += 1
            if calls == 3:
                raise WorkbenchError('VERIFY_FAILED', 'injected disk read-back failure')
            return read_source(*args)

        with patch.object(self.workspace, '_replacement_source', side_effect=fail_readback):
            self.expect_code('VERIFY_FAILED', self.replace)
        self.assert_original()
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o640)

    def test_temporary_fsync_failure_leaves_original_and_no_temporary_files(self):
        with patch('hwp_workbench.workspace.os.fsync', side_effect=OSError('injected temporary sync failure')):
            self.expect_code('WRITE_FAILED', self.replace)
        self.assert_original()

    def test_recovery_cleanup_failure_restores_original(self):
        unlink = os.unlink
        failed = False

        def fail_once(*args, **kwargs):
            nonlocal failed
            if not failed:
                failed = True
                raise OSError('injected recovery cleanup failure')
            return unlink(*args, **kwargs)

        with patch('hwp_workbench.workspace.os.unlink', side_effect=fail_once):
            self.expect_code('WRITE_FAILED', self.replace)
        self.assert_original()


class ChangedDocumentTests(ServiceSaveFixture, unittest.TestCase):
    def test_changed_source_is_refused_before_parsing_new_bytes(self):
        self.path.write_bytes(b'external change that is no longer a native document')
        self.expect_code('SOURCE_CHANGED', self.save)
        self.assertEqual(self.path.read_bytes(), b'external change that is no longer a native document')
        self.assertFalse((self.root / '.hwp-workbench').exists())
        self.assertEqual(list(self.root.rglob('*.tmp')), [])
