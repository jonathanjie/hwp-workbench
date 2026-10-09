"""B2 agent workflows over self-authored fixtures, without Hancom claims."""
import copy
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from fixtures import make_hwp, make_hwpx
from hwp_workbench.cli import _parser
from hwp_workbench.document import Document
from hwp_workbench.errors import WorkbenchError
from hwp_workbench.mcp import serve
from hwp_workbench.service import invoke

PROJECT = Path(__file__).resolve().parents[1]


class AgentLinkCase(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        (self.root / 'form.hwpx').write_bytes(make_hwpx())
        (self.root / 'legacy.hwp').write_bytes(make_hwp())

    def tearDown(self):
        self.temp.cleanup()

    def cli(self, *arguments, home=None):
        environment = {**os.environ, 'PYTHONPATH': str(PROJECT / 'engine')}
        if home is not None:
            environment['HWP_WORKBENCH_SKILL_HOME'] = str(home)
        return subprocess.run([sys.executable, '-m', 'hwp_workbench', *arguments],
                              env=environment, capture_output=True, text=True, timeout=10)

    def mcp(self, method, arguments):
        messages = [
            {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize',
             'params': {'protocolVersion': '2025-11-25'}},
            {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
            {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call',
             'params': {'name': 'hwp_' + method, 'arguments': arguments}},
        ]
        output = io.StringIO()
        serve(self.root, io.StringIO('\n'.join(map(json.dumps, messages)) + '\n'), output)
        return json.loads(output.getvalue().splitlines()[-1])

    def assert_code(self, code, action):
        with self.assertRaises(WorkbenchError) as caught:
            action()
        self.assertEqual(caught.exception.code, code)


class ErgonomicsTests(AgentLinkCase):
    def test_short_references_cli_mcp_and_patch_operations(self):
        for source in ['form.hwpx', 'legacy.hwp']:
            with self.subTest(source=source):
                read = invoke(self.root, 'read', {'path': source, 'limit': 1})
                node_id = read['references']['s0:t0']
                document = Document((self.root / source).read_bytes())
                text = document.by_id[node_id].text.replace('검토', '확정')
                arguments = {'path': source, 'nodeId': 's0:t0', 'text': text}
                result = invoke(self.root, 'set', arguments)
                self.assertEqual(result['patch']['operations'][0]['nodeId'], node_id)
                self.assertEqual(self.mcp('set', arguments)['result']['structuredContent'], result)
                process = self.cli('set', source, '--root', str(self.root), '--node-id', 's0:t0', '--text', text)
                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual(json.loads(process.stdout), result)
                scoped = invoke(self.root, 'replace', {'path': source, 'find': '검토',
                                                     'replaceWith': '확정', 'nodeIds': ['s0:t0']})
                self.assertEqual(scoped['patch']['operations'][0]['nodeId'], node_id)
                process = self.cli('replace', source, '--root', str(self.root), '--find', '검토',
                                   '--replace-with', '확정', '--node-id', 's0:t0')
                self.assertEqual(json.loads(process.stdout), scoped)
                patch = copy.deepcopy(result['patch'])
                patch['operations'][0]['nodeId'] = 's0:t0'
                planned = invoke(self.root, 'plan', {'path': source, 'patch': patch})
                self.assertEqual(planned['changes'], result['changes'])
                self.assertEqual(self.mcp('plan', {'path': source, 'patch': patch})['result']['structuredContent'], planned)
                (self.root / 'patch.json').write_text(json.dumps(patch))
                process = self.cli('apply', source, '--root', str(self.root), '--patch', 'patch.json',
                                   '-o', 'new-' + source, '--acknowledge-risks')
                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual(Document((self.root / ('new-' + source)).read_bytes()).by_id[node_id].text, text)

    def test_alias_duplicate_and_stale_text_are_refused_atomically(self):
        preview = invoke(self.root, 'set', {'path': 'form.hwpx', 'nodeId': 's0:t0', 'text': 'new'})
        patch = preview['patch']
        duplicate = copy.deepcopy(patch['operations'][0])
        duplicate['nodeId'] = 's0:t0'
        patch['operations'].append(duplicate)
        doc = Document((self.root / 'form.hwpx').read_bytes())
        self.assert_code('BAD_PATCH', lambda: doc.patch(patch))
        self.assertEqual(doc.edits, {})
        patch['operations'] = [duplicate]
        duplicate['expectedText'] = 'wrong'
        self.assert_code('STALE_TEXT', lambda: doc.patch(patch))
        self.assertEqual(doc.edits, {})
        self.assert_code('NODE_NOT_FOUND', lambda: invoke(self.root, 'set',
                         {'path': 'form.hwpx', 'nodeId': 's9:t999', 'text': 'new'}))

    def test_read_text_view_and_json_default(self):
        expected = invoke(self.root, 'read', {'path': 'form.hwpx', 'limit': 1})
        process = self.cli('read', 'form.hwpx', '--root', str(self.root), '--limit', '1', '--format', 'text')
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(process.stdout, expected['text'] + '\nsha256=' + expected['sha256'] + ' nextOffset=1\n')
        process = self.cli('read', 'form.hwpx', '--root', str(self.root), '--limit', '1')
        self.assertEqual(json.loads(process.stdout), expected)
        self.assertEqual(_parser().parse_args(['read', 'form.hwpx']).format, 'json')

    def test_search_positional_query_and_legacy_flag(self):
        positional = self.cli('search', 'form.hwpx', '검토', '--root', str(self.root))
        flagged = self.cli('search', 'form.hwpx', '--query', '검토', '--root', str(self.root))
        self.assertEqual(positional.returncode, 0, positional.stderr)
        self.assertEqual(positional.stdout, flagged.stdout)
        ambiguous = self.cli('search', 'form.hwpx', '검토', '--query', 'other', '--root', str(self.root))
        self.assertEqual(json.loads(ambiguous.stderr)['error']['code'], 'BAD_ARGUMENTS')

    def test_fill_repeated_fields_and_json(self):
        fields = {'기관명': 'New=Org', '담당자': 'Jane'}
        (self.root / 'fields.json').write_text(json.dumps(fields))
        repeated = self.cli('fill', 'form.hwpx', '--root', str(self.root),
                            '--field', '기관명=New=Org', '--field', '담당자=Jane')
        original = self.cli('fill', 'form.hwpx', '--root', str(self.root), '--json', 'fields.json')
        self.assertEqual(repeated.returncode, 0, repeated.stderr)
        self.assertEqual(repeated.stdout, original.stdout)
        for flags in [[], ['--field', 'broken'], ['--field', '=value'],
                      ['--field', '기관명=A', '--field', '기관명=B']]:
            process = self.cli('fill', 'form.hwpx', '--root', str(self.root), *flags)
            self.assertEqual(json.loads(process.stderr)['error']['code'], 'BAD_ARGUMENTS')

    def test_every_subcommand_and_argument_has_help(self):
        parser = _parser()
        subcommands = next(action for action in parser._actions if action.dest == 'command')
        descriptions = {action.dest for action in subcommands._choices_actions if action.help}
        self.assertEqual(descriptions, set(subcommands.choices))
        for name, command in subcommands.choices.items():
            for action in command._actions:
                with self.subTest(command=name, argument=action.dest):
                    self.assertTrue(action.help)


class ProposalTests(AgentLinkCase):
    def preview(self, source='form.hwpx'):
        return invoke(self.root, 'replace', {'path': source, 'find': '검토', 'replaceWith': '확정'})

    def propose(self, patch, source='form.hwpx', **options):
        return invoke(self.root, 'propose', {'path': source, 'patch': patch, 'title': 'Review text', **options})

    def test_proposal_envelope_and_full_patch_cli_mcp(self):
        import re

        for source in ['form.hwpx', 'legacy.hwp']:
            with self.subTest(source=source):
                preview = self.preview(source)
                result = self.propose(preview, source)
                self.assertEqual(result['changes'], preview['changes'])
                self.assertRegex(result['proposal'], r'^\.hwp-workbench/proposals/' + re.escape(source) + r'\.\d{8}T\d{6}Z-[a-f0-9]{6}\.json$')
                proposal = json.loads((self.root / result['proposal']).read_bytes())
                self.assertEqual(proposal['schemaVersion'], 1)
                self.assertEqual(proposal['kind'], 'hwp-workbench-proposal')
                self.assertEqual(proposal['document'], {'name': source, 'sha256': preview['patch']['baseSha256']})
                self.assertEqual(proposal['patch'], preview['patch'])
                self.assertEqual(proposal['title'], 'Review text')
                self.assertEqual(proposal['author'], 'agent')
                (self.root / 'preview.json').write_text(json.dumps(preview))
                process = self.cli('propose', source, '--root', str(self.root), '--patch', 'preview.json',
                                   '--title', 'CLI review', '--author', 'codex')
                self.assertEqual(process.returncode, 0, process.stderr)
                cli_result = json.loads(process.stdout)
                self.assertEqual(json.loads((self.root / cli_result['proposal']).read_bytes())['author'], 'codex')
                response = self.mcp('propose', {'path': source, 'patch': preview,
                                              'title': 'MCP review', 'notes': 'Check the layout.'})
                self.assertFalse(response['result']['isError'])
                record = json.loads((self.root / response['result']['structuredContent']['proposal']).read_bytes())
                self.assertEqual(record['notes'], 'Check the layout.')
                self.assertFalse((self.root / '.hwp-workbench' / (source + '.session.json')).exists())

    def test_complete_validation_before_creating_directories(self):
        patch = self.preview()['patch']
        cases = [('baseSha256', 'wrong', 'STALE_DOCUMENT')]
        for key, value, code in cases:
            invalid = copy.deepcopy(patch)
            invalid[key] = value
            self.assert_code(code, lambda: self.propose(invalid))
        for key, value, code in [('expectedText', 'wrong', 'STALE_TEXT'), ('op', 'delete', 'BAD_PATCH'),
                                 ('text', 'bad\ntext', 'CONTROL_TEXT'), ('nodeId', 'missing', 'NODE_NOT_FOUND')]:
            invalid = copy.deepcopy(patch)
            invalid['operations'][0][key] = value
            self.assert_code(code, lambda: self.propose(invalid))
        self.assertFalse((self.root / '.hwp-workbench').exists())

    def test_collision_refused_without_overwrite(self):
        from unittest.mock import patch

        with patch('hwp_workbench.agent_link.secrets.token_hex', return_value='abcdef'):
            with patch('hwp_workbench.agent_link._utc_now', return_value='2026-10-09T10:20:30Z'):
                result = self.propose(self.preview()['patch'])
                before = (self.root / result['proposal']).read_bytes()
                self.assert_code('OUTPUT_EXISTS', lambda: self.propose(self.preview()['patch']))
                self.assertEqual((self.root / result['proposal']).read_bytes(), before)

    def test_confined_nested_path_and_symlink_directories(self):
        (self.root / 'nested').mkdir()
        (self.root / 'nested/form.hwpx').write_bytes(make_hwpx())
        preview = self.preview('nested/form.hwpx')
        result = self.propose(preview, str(self.root / 'nested/form.hwpx'))
        self.assertTrue(result['proposal'].startswith('nested/.hwp-workbench/proposals/'))
        self.assert_code('OUTSIDE_WORKSPACE', lambda: self.propose(preview, '../form.hwpx'))
        with tempfile.TemporaryDirectory() as outside:
            (self.root / '.hwp-workbench').symlink_to(outside, target_is_directory=True)
            self.assert_code('UNSAFE_PATH', lambda: self.propose(self.preview()))
            self.assertEqual(list(Path(outside).iterdir()), [])
            (self.root / '.hwp-workbench').unlink()
            (self.root / '.hwp-workbench').mkdir()
            (self.root / '.hwp-workbench/proposals').symlink_to(outside, target_is_directory=True)
            self.assert_code('UNSAFE_PATH', lambda: self.propose(self.preview()))
            self.assertEqual(list(Path(outside).iterdir()), [])

    def test_proposal_limits_and_short_reference_normalization(self):
        patch = self.preview()['patch']
        patch['operations'][0]['nodeId'] = 's0:t0'
        result = self.propose(patch)
        record = json.loads((self.root / result['proposal']).read_bytes())
        self.assertEqual(record['patch']['operations'][0]['nodeId'], Document(make_hwpx()).nodes[0].id)
        self.assert_code('LIMIT', lambda: self.propose(patch, notes='x' * (2 * 1024 * 1024)))
        patch['operations'] *= 1001
        self.assert_code('BAD_PATCH', lambda: self.propose(patch))


class SessionTests(AgentLinkCase):
    def link(self):
        folder = self.root / '.hwp-workbench'
        folder.mkdir(exist_ok=True)
        document = Document((self.root / 'form.hwpx').read_bytes())
        record = {'schemaVersion': 1, 'app': 'hwp-workbench', 'appVersion': '0.1.0-alpha.1',
                  'document': {'name': 'form.hwpx', 'format': 'hwpx', 'sha256': document.hash},
                  'selection': {'nodeId': document.nodes[0].id, 'text': document.nodes[0].text},
                  'staged': None, 'updatedAt': '2026-10-09T10:20:30Z'}
        path = folder / 'form.hwpx.session.json'
        path.write_text(json.dumps(record))
        return path, record

    def proposal(self):
        preview = invoke(self.root, 'replace', {'path': 'form.hwpx', 'find': '검토', 'replaceWith': '확정'})
        result = invoke(self.root, 'propose', {'path': 'form.hwpx', 'patch': preview, 'title': 'Check text'})
        return self.root / result['proposal']

    def session(self):
        return invoke(self.root, 'session', {'path': 'form.hwpx'})

    def test_absent_session_is_read_only_cli_mcp_parity(self):
        expected = {'linked': False, 'session': None, 'proposals': []}
        self.assertEqual(self.session(), expected)
        process = self.cli('session', 'form.hwpx', '--root', str(self.root))
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(json.loads(process.stdout), expected)
        self.assertEqual(self.mcp('session', {'path': 'form.hwpx'})['result']['structuredContent'], expected)
        self.assertFalse((self.root / '.hwp-workbench').exists())

    def test_desktop_session_and_per_document_isolation(self):
        session_path, session = self.link()
        proposal_path = self.proposal()
        result = self.session()
        self.assertTrue(result['linked'])
        self.assertEqual(result['session'], session)
        self.assertEqual(result['proposals'], [{'file': str(proposal_path.relative_to(self.root)),
                                               'title': 'Check text', 'author': 'agent',
                                               'createdAt': json.loads(proposal_path.read_text())['createdAt'],
                                               'operations': 3, 'status': 'pending'}])
        self.assertEqual(invoke(self.root, 'session', {'path': 'legacy.hwp'}),
                         {'linked': False, 'session': None, 'proposals': []})
        session_path.write_text(json.dumps(session | {'document': session['document'] | {'sha256': '0' * 64}}))
        self.assertFalse(self.session()['linked'])
        self.assertIsNone(self.session()['session'])

    def test_outcome_binds_exact_proposal_bytes_and_node_partition(self):
        from hwp_workbench.document import sha
        proposal_path = self.proposal()
        content = proposal_path.read_bytes()
        proposal = json.loads(content)
        ids = [operation['nodeId'] for operation in proposal['patch']['operations']]
        outcome_path = proposal_path.with_suffix('.outcome.json')
        valid = {'schemaVersion': 1, 'status': 'accepted', 'decidedAt': 'now',
                 'proposalSha256': sha(content), 'acceptedNodeIds': ids, 'declinedNodeIds': []}
        invalid = [
            {key: value for key, value in valid.items() if key != 'proposalSha256'},
            valid | {'proposalSha256': '0' * 64},
            valid | {'acceptedNodeIds': ['unrelated-old-node']},
            valid | {'acceptedNodeIds': ids[:-1]},
            valid | {'acceptedNodeIds': ids + [ids[0]]},
            valid | {'declinedNodeIds': [ids[0]]},
        ]
        for outcome in invalid:
            with self.subTest(outcome=outcome):
                outcome_path.write_text(json.dumps(outcome))
                self.assertEqual(self.session()['proposals'][0]['status'], 'needsReview')
        outcome_path.write_text(json.dumps(valid))
        self.assertEqual(self.session()['proposals'][0]['status'], 'accepted')
        proposal['patch']['operations'][0]['text'] = 'replacement proposal text'
        proposal_path.write_text(json.dumps(proposal))
        self.assertEqual(self.session()['proposals'][0]['status'], 'needsReview')

    def test_accepted_declined_partial_outcomes_and_unreadable(self):
        proposal_path = self.proposal()
        from hwp_workbench.document import sha
        content = proposal_path.read_bytes()
        ids = [operation['nodeId'] for operation in json.loads(content)['patch']['operations']]
        outcome_path = proposal_path.with_suffix('.outcome.json')
        for status in ['accepted', 'declined', 'partial']:
            outcome = {'schemaVersion': 1, 'status': status,
                       'proposalSha256': sha(content),
                       'acceptedNodeIds': ids if status == 'accepted' else (ids[:1] if status == 'partial' else []),
                       'declinedNodeIds': ids if status == 'declined' else (ids[1:] if status == 'partial' else []),
                       'decidedAt': '2026-10-09T10:20:30Z'}
            outcome_path.write_text(json.dumps(outcome))
            self.assertEqual(self.session()['proposals'][0]['status'], status)
        outcome_path.write_text('{bad')
        self.assertEqual(self.session()['proposals'][0]['status'], 'needsReview')
        self.assertEqual(len(self.session()['proposals']), 1)

    def test_broken_oversized_symlink_and_nonregular_files_are_unreadable(self):
        session_path, _ = self.link()
        for data in ['{bad', '[]', 'null', '{"schemaVersion":1}', '[' * 1100]:
            session_path.write_text(data)
            self.assertEqual(self.session()['session'], None)
        proposal_path = self.proposal()
        for data in ['{bad', '[]', 'x' * (2 * 1024 * 1024 + 1)]:
            proposal_path.write_text(data)
            self.assertEqual(self.session()['proposals'][0]['status'], 'unreadable')
        proposal_path.unlink()
        proposal_path.symlink_to(self.root / 'form.hwpx')
        self.assertEqual(self.session()['proposals'][0]['status'], 'unreadable')
        proposal_path.unlink()
        os.mkfifo(proposal_path)
        self.assertEqual(self.session()['proposals'][0]['status'], 'unreadable')

    def test_reads_at_most_200_proposals_with_per_file_size_limit(self):
        from unittest.mock import patch
        from hwp_workbench.workspace import Workspace

        proposal_path = self.proposal()
        folder = proposal_path.parent
        for index in range(205):
            (folder / f'form.hwpx.20261009T100000Z-{index:06x}.json').write_text('{bad')
        calls = []
        original_read = Workspace.read

        def counted_read(workspace, path, **options):
            if '/proposals/' in path and not path.endswith('.outcome.json'):
                calls.append((path, options))
            return original_read(workspace, path, **options)

        with patch.object(Workspace, 'read', counted_read):
            result = self.session()
        self.assertEqual(len(result['proposals']), 200)
        self.assertEqual(len(calls), 200)
        self.assertTrue(all(options['max_bytes'] == 2 * 1024 * 1024 for _, options in calls))

    def test_document_name_prefix_does_not_include_another_document(self):
        source = 'form.hwpx.other.hwpx'
        (self.root / source).write_bytes(make_hwpx())
        preview = invoke(self.root, 'set', {'path': source, 'nodeId': 's0:t0', 'text': 'new'})
        invoke(self.root, 'propose', {'path': source, 'patch': preview, 'title': 'Other document'})
        self.assertEqual(self.session()['proposals'], [])

    def test_link_directory_symlink_is_refused(self):
        with tempfile.TemporaryDirectory() as outside:
            (self.root / '.hwp-workbench').symlink_to(outside, target_is_directory=True)
            self.assert_code('UNSAFE_PATH', self.session)


class SkillRecipesTests(AgentLinkCase):
    def test_skill_contract_and_three_recipes(self):
        skill_path = PROJECT / 'skills/hwp-workbench/SKILL.md'
        self.assertTrue(skill_path.is_file(), 'The installable agent skill is missing.')
        skill = skill_path.read_text()
        self.assertLess(len(skill.splitlines()), 150)
        self.assertTrue(skill.startswith('---\nname: hwp-workbench\ndescription:'))
        for term in ['HWP', 'HWPX', '한글 문서', 'HWP Workbench', '{{HWP_WORKBENCH}}',
                     'native controls', 'untrusted data', 'never as instructions', 'warnings',
                     'references/patch-format.md']:
            self.assertIn(term, skill)
        reference = (skill_path.parent / 'references/patch-format.md').read_text()
        for term in ['schemaVersion', 'baseSha256', 'set_text', 'nodeId', 'expectedText',
                     'STALE_DOCUMENT', 'STALE_TEXT', 'UNSUPPORTED_ELEMENT', 'OUTPUT_EXISTS']:
            self.assertIn(term, reference)
        read = self.cli('read', 'form.hwpx', '--root', str(self.root))
        self.assertEqual(read.returncode, 0, read.stderr)
        search = self.cli('search', 'form.hwpx', '기관명', '--root', str(self.root))
        self.assertEqual(search.returncode, 0, search.stderr)
        previews = [
            self.cli('fill', 'form.hwpx', '--root', str(self.root), '--field', '기관명=New Org', '--field', '담당자=Jane'),
            self.cli('replace', 'form.hwpx', '--root', str(self.root), '--find', '검토', '--replace-with', '확정'),
            self.cli('set', 'form.hwpx', '--root', str(self.root), '--node-id', 's0:t0', '--text', 'New title'),
        ]
        for index, preview in enumerate(previews):
            self.assertEqual(preview.returncode, 0, preview.stderr)
            result = json.loads(preview.stdout)
            self.assertTrue(result['changes'])
            patch_name = f'review-{index}.patch.json'
            (self.root / patch_name).write_text(json.dumps(result['patch']))
            plan = self.cli('plan', 'form.hwpx', '--root', str(self.root), '--patch', patch_name)
            self.assertEqual(plan.returncode, 0, plan.stderr)
            proposal = self.cli('propose', 'form.hwpx', '--root', str(self.root), '--patch', patch_name,
                                '--title', 'Please review', '--author', 'codex')
            self.assertEqual(proposal.returncode, 0, proposal.stderr)
            apply = self.cli('apply', 'form.hwpx', '--root', str(self.root), '--patch', patch_name,
                             '-o', f'edited-{index}.hwpx', '--acknowledge-risks')
            self.assertEqual(apply.returncode, 0, apply.stderr)
        report = self.cli('session', 'form.hwpx', '--root', str(self.root))
        self.assertEqual(report.returncode, 0, report.stderr)
        self.assertEqual(len(json.loads(report.stdout)['proposals']), 3)


class SkillInstallTests(AgentLinkCase):
    def setUp(self):
        super().setUp()
        self.home = self.root / 'home'
        self.home.mkdir()

    def skill_cli(self, *arguments):
        return self.cli('skill', *arguments, home=self.home)

    def test_install_status_uninstall_and_path_substitution(self):
        initial = self.skill_cli('status')
        self.assertEqual(initial.returncode, 0, initial.stderr)
        self.assertFalse(json.loads(initial.stdout)['claude']['installed'])
        install = self.skill_cli('install')
        self.assertEqual(install.returncode, 0, install.stderr)
        for target in ['claude', 'codex']:
            folder = self.home / ('.' + target) / 'skills/hwp-workbench'
            skill = (folder / 'SKILL.md').read_text()
            reference = (folder / 'references/patch-format.md').read_text()
            self.assertIn(str(PROJECT / 'scripts/hwp-workbench'), skill)
            self.assertNotIn('{{HWP_WORKBENCH}}', skill + reference)
            self.assertIn('<!-- installed by hwp-workbench 0.1.0-alpha.1 -->', skill)
            status = json.loads(self.skill_cli('status').stdout)[target]
            self.assertEqual(status, {'installed': True, 'version': '0.1.0-alpha.1', 'path': str(folder)})
        # Updating an owned installation keeps the recipes and reference available.
        self.assertEqual(self.skill_cli('install').returncode, 0)
        uninstall = self.skill_cli('uninstall')
        self.assertEqual(uninstall.returncode, 0, uninstall.stderr)
        status = json.loads(self.skill_cli('status').stdout)
        self.assertFalse(status['claude']['installed'])
        self.assertFalse(status['codex']['installed'])
        self.assertFalse((self.home / '.codex/skills/hwp-workbench').exists())

    def test_target_and_dry_run_do_not_touch_other_folders(self):
        dry_run = self.skill_cli('install', '--dry-run')
        self.assertEqual(dry_run.returncode, 0, dry_run.stderr)
        self.assertEqual(list(self.home.iterdir()), [])
        install = self.skill_cli('install', '--target', 'claude')
        self.assertEqual(install.returncode, 0, install.stderr)
        self.assertTrue((self.home / '.claude/skills/hwp-workbench/SKILL.md').is_file())
        self.assertFalse((self.home / '.codex').exists())

    def test_unmarked_folder_refused_and_preserved_on_uninstall(self):
        folder = self.home / '.codex/skills/hwp-workbench'
        folder.mkdir(parents=True)
        (folder / 'SKILL.md').write_text('User-owned skill')
        for flags in [[], ['--dry-run']]:
            process = self.skill_cli('install', *flags)
            self.assertEqual(process.returncode, 2)
            self.assertEqual(json.loads(process.stderr)['error']['code'], 'SKILL_NOT_OWNED')
            self.assertFalse((self.home / '.claude').exists(), 'Preflight must check all targets before writing.')
        uninstall = self.skill_cli('uninstall')
        self.assertEqual(uninstall.returncode, 0, uninstall.stderr)
        self.assertEqual((folder / 'SKILL.md').read_text(), 'User-owned skill')

    def test_symlink_parent_folder_and_file_are_never_followed(self):
        with tempfile.TemporaryDirectory() as outside:
            outside_path = Path(outside)
            (self.home / '.claude').symlink_to(outside_path, target_is_directory=True)
            install = self.skill_cli('install', '--target', 'claude')
            self.assertEqual(install.returncode, 2)
            self.assertEqual(list(outside_path.iterdir()), [])
            (self.home / '.claude').unlink()
            skills = self.home / '.claude/skills'
            skills.mkdir(parents=True)
            folder = skills / 'hwp-workbench'
            folder.symlink_to(outside_path, target_is_directory=True)
            self.assertEqual(self.skill_cli('install', '--target', 'claude').returncode, 2)
            self.skill_cli('uninstall')
            self.assertTrue(folder.is_symlink())
            folder.unlink()
            folder.mkdir()
            target = outside_path / 'SKILL.md'
            target.write_text('<!-- installed by hwp-workbench 0.1.0-alpha.1 -->\nExternal')
            (folder / 'SKILL.md').symlink_to(target)
            self.assertEqual(self.skill_cli('install', '--target', 'claude').returncode, 2)
            self.skill_cli('uninstall')
            self.assertTrue((folder / 'SKILL.md').is_symlink())
            self.assertIn('External', target.read_text())


class LinkSafetyRegressionTests(AgentLinkCase):
    def test_temporary_name_collision_preserves_existing_file(self):
        from unittest.mock import patch
        from hwp_workbench.workspace import Workspace

        temporary = self.root / '.hwp-workbench-fixed.tmp'
        temporary.write_bytes(b'user data')
        with patch('hwp_workbench.workspace.secrets.token_hex', return_value='fixed'):
            self.assert_code('OUTPUT_EXISTS', lambda: Workspace(self.root).write_new('new.json', b'new'))
        self.assertEqual(temporary.read_bytes(), b'user data')
        self.assertFalse((self.root / 'new.json').exists())

    def test_null_outcome_requires_review(self):
        preview = invoke(self.root, 'set', {'path': 'form.hwpx', 'nodeId': 's0:t0', 'text': 'new'})
        result = invoke(self.root, 'propose', {'path': 'form.hwpx', 'patch': preview, 'title': 'Review'})
        proposal = self.root / result['proposal']
        proposal.with_suffix('.outcome.json').write_text('null')
        report = invoke(self.root, 'session', {'path': 'form.hwpx'})
        self.assertEqual(report['proposals'][0]['status'], 'needsReview')

    def test_marked_skill_is_backed_up_before_replacement(self):
        home = self.root / 'home'
        home.mkdir()
        self.assertEqual(self.cli('skill', 'install', '--target', 'codex', home=home).returncode, 0)
        folder = home / '.codex/skills/hwp-workbench'
        (folder / 'local-notes.txt').write_text('Keep this user configuration.')
        process = self.cli('skill', 'install', '--target', 'codex', home=home)
        self.assertEqual(process.returncode, 0, process.stderr)
        report = json.loads(process.stdout)
        self.assertEqual(len(report['backups']), 1)
        backup = Path(report['backups'][0])
        self.assertEqual((backup / 'local-notes.txt').read_text(), 'Keep this user configuration.')
