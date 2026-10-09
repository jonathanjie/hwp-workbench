"""Agent operations over self-authored fixtures; no Hancom certification."""
import unittest
import unicodedata

from fixtures import make_hwp, make_hwpx
from hwp_workbench import agent_ops
from hwp_workbench.document import Document, HP
from hwp_workbench.errors import WorkbenchError


class AgentOperationTests(unittest.TestCase):
    def documents(self):
        return [Document(make_hwp()), Document(make_hwpx())]

    def test_read_paging_and_reference_mapping(self):
        for doc in self.documents():
            with self.subTest(format=doc.format):
                result = agent_ops.read(doc, limit=1)
                self.assertEqual(result['sha256'], doc.hash)
                self.assertEqual(result['total'], len(doc.nodes))
                self.assertEqual(result['references']['s0:t0'], doc.nodes[0].id)
                self.assertIn('[s0:t0]', result['text'])
                self.assertEqual(result['nextOffset'], 1 if len(doc.nodes) > 1 else None)
                self.assertTrue(all(isinstance(code, str) for code in result['warnings']))
                self.assertEqual(agent_ops.read(doc, offset=len(doc.nodes))['text'], '')

    def test_read_tables_and_read_only_nodes(self):
        doc = Document(make_hwpx())
        doc.nodes[4].editable = False
        result = agent_ops.read(doc)
        self.assertIn('| [s0:t3] 항목 / Item | [s0:t4] (read-only) 내용 / Details |', result['text'])

    def test_search_offsets_case_context_and_limit(self):
        for doc in self.documents():
            with self.subTest(format=doc.format):
                result = agent_ops.search(doc, '검토', limit=1)
                self.assertEqual(len(result['matches']), 1)
                match = result['matches'][0]
                node = doc.by_id[match['nodeId']]
                self.assertEqual(node.text[match['start']:match['end']], '검토')
                self.assertIn('검토', match['context'])
                self.assertTrue(match['editable'])
        doc = Document(make_hwpx())
        self.assertEqual(len(agent_ops.search(doc, 'internal')['matches']), 1)
        self.assertEqual(agent_ops.search(doc, 'internal', case_sensitive=True)['matches'], [])
        self.assertEqual(len(agent_ops.search(doc, r'J[oa]nathan', regex=True)['matches']), 1)

    def test_regex_limits_and_invalid_arguments(self):
        doc = Document(make_hwpx())
        for query in ['(a+)+$', 'a|aa', r'(a)\1', 'a*', 'a{1,100}', 'a' * 257, '[']:
            with self.subTest(query=query), self.assertRaises(WorkbenchError):
                agent_ops.search(doc, query, regex=True)
        for options in [{'limit': 0}, {'limit': True}, {'offset': -1}]:
            with self.subTest(options=options), self.assertRaises(WorkbenchError):
                agent_ops.read(doc, **options)

    def test_replace_patch_is_pure_and_allows_hwp_length_change(self):
        for doc in self.documents():
            with self.subTest(format=doc.format):
                result = agent_ops.replace(doc, '검토', '확정')
                self.assertGreater(len(result['changes']), 0)
                self.assertEqual(result['patch']['baseSha256'], doc.hash)
                self.assertEqual(doc.edits, {})
                doc.patch(result['patch'])
                self.assertIn('확정', Document(doc.serialize(True)).nodes[0].text)
        doc = Document(make_hwp())
        result = agent_ops.replace(doc, '검토', '확정됨')
        self.assertEqual(result['skipped'], [])
        doc.patch(result['patch'])
        self.assertIn('확정됨', Document(doc.serialize(True)).nodes[0].text)

    def test_replace_read_only_scope_and_occurrence_limit(self):
        doc = Document(make_hwpx())
        doc.nodes[0].editable = False
        result = agent_ops.replace(doc, '검토', '확정')
        self.assertEqual(result['skipped'][0]['code'], 'UNSUPPORTED_ELEMENT')
        self.assertTrue(result['changes'])
        node = next(node for node in doc.nodes if '검토' in node.text and node.editable)
        scoped = agent_ops.replace(doc, '검토', '확정', node_ids=[node.id], limit=1)
        self.assertEqual([change['nodeId'] for change in scoped['changes']], [node.id])
        with self.assertRaises(WorkbenchError):
            agent_ops.replace(doc, '검토', '확정', node_ids=['missing'])
        section = f'<hp:p xmlns:hp="{HP}"><hp:run><hp:t>aaa</hp:t></hp:run></hp:p>'
        repeated = Document(make_hwpx(section=section))
        result = agent_ops.replace(repeated, 'a', 'b', limit=2)
        self.assertEqual(result['changes'][0]['after'], 'bba')

    def test_fill_normalises_fixture_labels_and_reports_hwp(self):
        doc = Document(make_hwpx())
        label = unicodedata.normalize('NFD', '기 관 명 :')
        result = agent_ops.fill(doc, {label: '새 기관', '담당자 / Contact': 'Jane', '없음': 'x'})
        self.assertEqual([item['after'] for item in result['filled']], ['새 기관', 'Jane'])
        self.assertEqual(result['unresolved'], ['없음'])
        self.assertEqual(doc.edits, {})
        doc.patch(result['patch'])
        reopened = Document(doc.serialize(True))
        self.assertIn('새 기관', [node.text for node in reopened.nodes])
        result = agent_ops.fill(Document(make_hwp()), {'기관명': 'x'})
        self.assertEqual(result['unresolved'], ['기관명'])
        self.assertEqual(result['unresolvedDetails'][0]['code'], 'HWP_TABLE_MAPPING_UNAVAILABLE')

    def test_fill_below_ambiguous_and_multi_run_cells(self):
        def cell(text):
            return f'<hp:tc><hp:p><hp:run><hp:t>{text}</hp:t></hp:run></hp:p></hp:tc>'
        section = f'<hp:tbl xmlns:hp="{HP}"><hp:tr>{cell("Label:")}</hp:tr><hp:tr>{cell("")}</hp:tr></hp:tbl>'
        result = agent_ops.fill(Document(make_hwpx(section=section)), {'Label': 'Value'})
        self.assertEqual(result['filled'][0]['after'], 'Value')
        section = f'<hp:tbl xmlns:hp="{HP}"><hp:tr>{cell("Label")}{cell("one")}</hp:tr><hp:tr>{cell("Label")}{cell("two")}</hp:tr></hp:tbl>'
        result = agent_ops.fill(Document(make_hwpx(section=section)), {'Label': 'Value'})
        self.assertEqual(result['unresolvedDetails'][0]['code'], 'AMBIGUOUS_LABEL')
        section = f'<hp:tbl xmlns:hp="{HP}"><hp:tr>{cell("Label")}<hp:tc><hp:p><hp:run><hp:t>a</hp:t><hp:t>b</hp:t></hp:run></hp:p></hp:tc></hp:tr></hp:tbl>'
        result = agent_ops.fill(Document(make_hwpx(section=section)), {'Label': 'Value'})
        self.assertEqual(result['filled'], [])
        self.assertEqual(result['unresolved'], ['Label'])

    def test_set_text_patch_and_native_validation(self):
        for doc in self.documents():
            with self.subTest(format=doc.format):
                node = doc.nodes[0]
                result = agent_ops.set_text(doc, node.id, node.text.replace('검토', '확정'))
                self.assertEqual(result['patch']['operations'][0]['expectedText'], node.text)
                self.assertEqual(len(result['changes']), 1)
                self.assertEqual(doc.edits, {})
                doc.patch(result['patch'])
                self.assertIn('확정', Document(doc.serialize(True)).nodes[0].text)
        with self.assertRaises(WorkbenchError) as caught:
            agent_ops.set_text(Document(make_hwp()), doc.nodes[0].id, 'short')
        self.assertEqual(caught.exception.code, 'NODE_NOT_FOUND')
        doc = Document(make_hwp())
        result = agent_ops.set_text(doc, doc.nodes[0].id, 'short')
        doc.patch(result['patch'])
        self.assertEqual(Document(doc.serialize(True)).nodes[0].text, 'short')

    def test_replace_surrogate_pairs_and_literal_regex_replacement(self):
        doc = Document(make_hwp())
        result = agent_ops.replace(doc, '검토', '😀')
        self.assertEqual(result['skipped'], [])
        doc.patch(result['patch'])
        self.assertIn('😀', Document(doc.serialize(True)).nodes[0].text)
        doc = Document(make_hwp())
        result = agent_ops.replace(doc, '검', '😀')
        self.assertEqual(result['skipped'], [])
        doc.patch(result['patch'])
        self.assertIn('😀', Document(doc.serialize(True)).nodes[0].text)
        doc = Document(make_hwpx())
        result = agent_ops.replace(doc, 'J[oa]nathan', r'\1', regex=True)
        self.assertEqual(result['changes'][0]['after'], r'\1 Jie')

    def test_search_regex_match_cap_and_zero_width_offsets(self):
        section = f'<hp:p xmlns:hp="{HP}"><hp:run><hp:t>{"a" * 1500}</hp:t></hp:run></hp:p>'
        doc = Document(make_hwpx(section=section))
        result = agent_ops.search(doc, '[a]', regex=True, limit=1000)
        self.assertEqual(len(result['matches']), 1000)
        self.assertEqual(result['matches'][-1]['start'], 999)
        self.assertTrue(result['limitReached'])
        result = agent_ops.search(doc, '^', regex=True)
        self.assertEqual(result['matches'][0]['start'], 0)
        self.assertEqual(result['matches'][0]['end'], 0)

    def test_fill_merged_rows_nested_tables_and_protected_values(self):
        def cell(text, span=1):
            return f'<hp:tc><hp:p><hp:run><hp:t>{text}</hp:t></hp:run></hp:p><hp:cellSpan colSpan="1" rowSpan="{span}"/></hp:tc>'

        section = f'<hp:tbl xmlns:hp="{HP}"><hp:tr>{cell("Opaque", 2)}{cell("Label:")}</hp:tr><hp:tr>{cell("")}</hp:tr></hp:tbl>'
        result = agent_ops.fill(Document(make_hwpx(section=section)), {'Label': 'Value'})
        self.assertEqual(result['filled'][0]['before'], '')
        nested = f'<hp:tbl><hp:tr>{cell("Inner")}{cell("old")}</hp:tr></hp:tbl>'
        section = f'<hp:tbl xmlns:hp="{HP}"><hp:tr>{cell("Outer")}<hp:tc>{nested}</hp:tc></hp:tr></hp:tbl>'
        result = agent_ops.fill(Document(make_hwpx(section=section)), {'Inner': 'new', 'Outer': 'bad'})
        self.assertEqual([item['label'] for item in result['filled']], ['Inner'])
        self.assertEqual(result['unresolved'], ['Outer'])
        doc = Document(make_hwpx({'META-INF/signatures.xml': b'<signature/>'}))
        result = agent_ops.fill(doc, {'기관명': 'new'})
        self.assertEqual(result['unresolved'], ['기관명'])
        self.assertEqual(result['filled'], [])
        self.assertIn('(read-only)', agent_ops.read(doc)['text'])

    def test_read_keeps_multiple_runs_in_one_grid_cell(self):
        section = f'<hp:tbl xmlns:hp="{HP}"><hp:tr><hp:tc><hp:p><hp:run><hp:t>first</hp:t><hp:t>second</hp:t></hp:run></hp:p></hp:tc><hp:tc><hp:p><hp:run><hp:t>third</hp:t></hp:run></hp:p></hp:tc></hp:tr></hp:tbl>'
        doc = Document(make_hwpx(section=section))
        self.assertEqual(agent_ops.read(doc)['text'], '| [s0:t0] first [s0:t1] second | [s0:t2] third |')

    def test_fill_uses_native_sparse_cell_addresses(self):
        def cell(text, row, column):
            return f'<hp:tc><hp:p><hp:run><hp:t>{text}</hp:t></hp:run></hp:p><hp:cellAddr rowAddr="{row}" colAddr="{column}"/></hp:tc>'

        section = f'<hp:tbl xmlns:hp="{HP}"><hp:tr>{cell("Label", 0, 1)}</hp:tr><hp:tr>{cell("Unrelated", 1, 0)}</hp:tr></hp:tbl>'
        result = agent_ops.fill(Document(make_hwpx(section=section)), {'Label': 'Value'})
        self.assertEqual(result['unresolved'], ['Label'])
        self.assertEqual(result['patch']['operations'], [])
        section = f'<hp:tbl xmlns:hp="{HP}"><hp:tr>{cell("Value", 0, 2)}{cell("Label", 0, 1)}</hp:tr></hp:tbl>'
        result = agent_ops.fill(Document(make_hwpx(section=section)), {'Label': 'New'})
        self.assertEqual(result['filled'][0]['before'], 'Value')

    def test_fill_refuses_invalid_native_cell_addresses(self):
        section = f'<hp:tbl xmlns:hp="{HP}"><hp:tr><hp:tc><hp:p><hp:run><hp:t>Label</hp:t></hp:run></hp:p><hp:cellAddr rowAddr="0" colAddr="invalid"/></hp:tc><hp:tc><hp:p><hp:run><hp:t>old</hp:t></hp:run></hp:p></hp:tc></hp:tr></hp:tbl>'
        result = agent_ops.fill(Document(make_hwpx(section=section)), {'Label': 'New'})
        self.assertEqual(result['unresolved'], ['Label'])
        self.assertEqual(result['unresolvedDetails'][0]['code'], 'TABLE_MAPPING_UNAVAILABLE')

    def test_search_context_is_short_for_long_literal_matches(self):
        text = 'before ' + 'x' * 500 + ' after'
        section = f'<hp:p xmlns:hp="{HP}"><hp:run><hp:t>{text}</hp:t></hp:run></hp:p>'
        doc = Document(make_hwpx(section=section))
        match = agent_ops.search(doc, 'x' * 500)['matches'][0]
        self.assertEqual((match['start'], match['end']), (7, 507))
        self.assertLessEqual(len(match['context']), 120)

    def test_fill_uses_native_span_instead_of_clamped_preview_span(self):
        def cell(text, row, span=1):
            return f'<hp:tc><hp:p><hp:run><hp:t>{text}</hp:t></hp:run></hp:p><hp:cellAddr rowAddr="{row}" colAddr="0"/><hp:cellSpan rowSpan="{span}" colSpan="1"/></hp:tc>'

        section = f'<hp:tbl xmlns:hp="{HP}"><hp:tr>{cell("Label", 0, 101)}</hp:tr><hp:tr>{cell("Value", 101)}</hp:tr></hp:tbl>'
        result = agent_ops.fill(Document(make_hwpx(section=section)), {'Label': 'New'})
        self.assertEqual(result['filled'][0]['before'], 'Value')


class AgentServiceTests(unittest.TestCase):
    def setUp(self):
        import tempfile
        from pathlib import Path

        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / 'form.hwpx').write_bytes(make_hwpx())
        (self.root / 'legacy.hwp').write_bytes(make_hwp())

    def tearDown(self):
        self.temp.cleanup()

    def test_service_agent_operations_and_compact_inspect(self):
        from hwp_workbench.service import invoke

        for path in ['form.hwpx', 'legacy.hwp']:
            with self.subTest(path=path):
                read = invoke(self.root, 'read', {'path': path, 'limit': 1})
                self.assertEqual(read['limit'], 1)
                search = invoke(self.root, 'search', {'path': path, 'query': '검토'})
                self.assertTrue(search['matches'])
                replace = invoke(self.root, 'replace', {'path': path, 'find': '검토', 'replaceWith': '확정'})
                self.assertTrue(replace['changes'])
                node_id = search['matches'][0]['nodeId']
                doc = Document((self.root / path).read_bytes())
                result = invoke(self.root, 'set', {'path': path, 'nodeId': node_id,
                                                'text': doc.by_id[node_id].text.replace('검토', '확정')})
                self.assertEqual(len(result['patch']['operations']), 1)
                fill = invoke(self.root, 'fill', {'path': path, 'fields': {'기관명': '새 기관'}})
                self.assertEqual(bool(fill['filled']), path.endswith('hwpx'))
                compact = invoke(self.root, 'inspect', {'path': path, 'compact': True})
                self.assertNotIn('view', compact)
                self.assertNotIn('parts', compact)
                self.assertIn('view', invoke(self.root, 'inspect', {'path': path}))

    def test_apply_through_replace_is_verified_and_never_overwrites(self):
        from hwp_workbench.service import invoke

        for path in ['form.hwpx', 'legacy.hwp']:
            with self.subTest(path=path):
                before = (self.root / path).read_bytes()
                args = {'path': path, 'find': '검토', 'replaceWith': '확정',
                        'output': 'new-' + path, 'acknowledgeRisks': True}
                result = invoke(self.root, 'replace', args)
                self.assertTrue(result['reopenedAndVerified'])
                output = self.root / args['output']
                self.assertIn('확정', Document(output.read_bytes()).nodes[0].text)
                saved = output.read_bytes()
                with self.assertRaises(WorkbenchError) as caught:
                    invoke(self.root, 'replace', args)
                self.assertEqual(caught.exception.code, 'OUTPUT_EXISTS')
                self.assertEqual(output.read_bytes(), saved)
                self.assertEqual((self.root / path).read_bytes(), before)
                args['output'] = 'no-ack-' + path
                args['acknowledgeRisks'] = False
                with self.assertRaises(WorkbenchError) as caught:
                    invoke(self.root, 'replace', args)
                self.assertEqual(caught.exception.code, 'RISK_ACK_REQUIRED')
                self.assertFalse((self.root / args['output']).exists())

    def test_fill_and_set_apply_through_shared_publication(self):
        from hwp_workbench.service import invoke

        cases = [
            ('fill', {'path': 'form.hwpx', 'fields': {'기관명': '새 기관'}, 'output': 'filled.hwpx'}),
            ('set', {'path': 'form.hwpx', 'output': 'set.hwpx'}),
            ('set', {'path': 'legacy.hwp', 'output': 'set.hwp'}),
        ]
        for method, args in cases:
            with self.subTest(method=method, path=args['path']):
                doc = Document((self.root / args['path']).read_bytes())
                if method == 'set':
                    args.update(nodeId=doc.nodes[0].id, text=doc.nodes[0].text.replace('검토', '확정'))
                args['acknowledgeRisks'] = True
                result = invoke(self.root, method, args)
                self.assertTrue(result['sourceUntouched'])
                reopened = Document((self.root / args['output']).read_bytes())
                self.assertEqual(reopened.hash, result['outputSha256'])
                for change in result['changes']:
                    self.assertEqual(reopened.by_id[change['nodeId']].text, change['after'])
                with self.assertRaises(WorkbenchError) as caught:
                    invoke(self.root, method, args)
                self.assertEqual(caught.exception.code, 'OUTPUT_EXISTS')

    def test_replace_refuses_changed_source_before_publication(self):
        from unittest.mock import patch
        from hwp_workbench.service import invoke
        from hwp_workbench.workspace import Workspace

        original_read = Workspace.read
        reads = 0
        newer = make_hwpx(section=f'<hp:p xmlns:hp="{HP}"><hp:run><hp:t>new source</hp:t></hp:run></hp:p>')

        def read_with_source_change(workspace, path):
            nonlocal reads
            reads += 1
            if reads == 2:
                (self.root / 'form.hwpx').write_bytes(newer)
            return original_read(workspace, path)

        args = {'path': 'form.hwpx', 'find': '검토', 'replaceWith': '확정',
                'output': 'stale.hwpx', 'acknowledgeRisks': True}
        with patch.object(Workspace, 'read', read_with_source_change):
            with self.assertRaises(WorkbenchError) as caught:
                invoke(self.root, 'replace', args)
        self.assertEqual(caught.exception.code, 'STALE_DOCUMENT')
        self.assertEqual((self.root / 'form.hwpx').read_bytes(), newer)
        self.assertFalse((self.root / 'stale.hwpx').exists())

    def test_empty_preview_cannot_write_and_output_stays_confined(self):
        from hwp_workbench.service import invoke

        args = {'path': 'form.hwpx', 'find': 'not present', 'replaceWith': 'x'}
        result = invoke(self.root, 'replace', args)
        self.assertEqual(result['patch']['operations'], [])
        args.update(output='empty.hwpx', acknowledgeRisks=True)
        with self.assertRaises(WorkbenchError) as caught:
            invoke(self.root, 'replace', args)
        self.assertEqual(caught.exception.code, 'BAD_PATCH')
        self.assertFalse((self.root / 'empty.hwpx').exists())
        args.update(find='검토', replaceWith='확정', output='../outside.hwpx')
        with self.assertRaises(WorkbenchError) as caught:
            invoke(self.root, 'replace', args)
        self.assertEqual(caught.exception.code, 'OUTSIDE_WORKSPACE')


class RecoveryTests(unittest.TestCase):
    def test_recovery_hints_for_stable_codes(self):
        cases = {
            'STALE_DOCUMENT': 'read',
            'STALE_TEXT': 'read',
            'HWP_LENGTH_CHANGE_BLOCKED': 'UTF-16',
            'OUTPUT_EXISTS': 'filename',
        }
        for code, action in cases.items():
            with self.subTest(code=code):
                error = WorkbenchError(code, 'test').as_dict()
                self.assertEqual(error['code'], code)
                self.assertIn(action, error['hint'])
                self.assertTrue(error['retryable'])
        error = WorkbenchError('VERIFY_FAILED', 'test').as_dict()
        self.assertFalse(error['retryable'])
