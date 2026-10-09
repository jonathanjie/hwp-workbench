import io,json,tempfile,unittest,subprocess,os,sys
from pathlib import Path
from fixtures import make_hwpx, make_hwp
from hwp_workbench.mcp import serve
from hwp_workbench.document import Document
from test_engine import patch

class MCPTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name);(self.root/'form.hwpx').write_bytes(make_hwpx())
    def tearDown(self):self.temp.cleanup()
    def run_messages(self,more,initialize=True):
        msgs=[]
        if initialize:msgs=[{'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'test','version':'1'}}},{'jsonrpc':'2.0','method':'notifications/initialized'}]
        msgs+=more;out=io.StringIO();serve(self.root,io.StringIO('\n'.join(json.dumps(x) for x in msgs)+'\n'),out);return [json.loads(x) for x in out.getvalue().splitlines()]
    def test_initialize_tools(self):
        replies = self.run_messages([{'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'}])
        self.assertEqual(replies[0]['result']['protocolVersion'], '2025-11-25')
        self.assertEqual(len(replies[1]['result']['tools']), 10)

    def test_inspect(self):
        r=self.run_messages([{'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'hwp_inspect','arguments':{'path':'form.hwpx'}}}]);self.assertFalse(r[-1]['result']['isError']);self.assertEqual(r[-1]['result']['structuredContent']['format'],'hwpx')
    def test_root_escape(self):
        r=self.run_messages([{'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'hwp_inspect','arguments':{'path':'/etc/passwd'}}}]);self.assertTrue(r[-1]['result']['isError'])
    def test_apply(self):
        d=Document((self.root/'form.hwpx').read_bytes());r=self.run_messages([{'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'hwp_apply','arguments':{'path':'form.hwpx','output':'new.hwpx','patch':patch(d),'acknowledgeRisks':True}}}]);self.assertFalse(r[-1]['result']['isError']);self.assertTrue((self.root/'new.hwpx').exists())
    def test_requires_initialized(self):
        r=self.run_messages([{'jsonrpc':'2.0','id':2,'method':'tools/list'}],False);self.assertEqual(r[0]['error']['code'],-32002)
    def test_unknown_tool(self):
        r=self.run_messages([{'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'exec','arguments':{}}}]);self.assertEqual(r[-1]['error']['code'],-32602)
    def test_wrong_argument_type(self):
        r=self.run_messages([{'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'hwp_inspect','arguments':[]}}]);self.assertEqual(r[-1]['error']['code'],-32602)
    def test_unknown_method(self):
        r=self.run_messages([{'jsonrpc':'2.0','id':2,'method':'eval'}]);self.assertEqual(r[-1]['error']['code'],-32601)
    def test_malformed_json(self):
        out=io.StringIO();serve(self.root,io.StringIO('invalid\n'),out);self.assertEqual(json.loads(out.getvalue())['error']['code'],-32700)
    def test_invalid_envelope(self):
        r=self.run_messages([['bad']],False);self.assertEqual(r[0]['error']['code'],-32600)
    def test_ping(self):
        r=self.run_messages([{'jsonrpc':'2.0','id':8,'method':'ping'}],False);self.assertEqual(r[0]['result'],{})
    def test_new_version_negotiates_supported_profile(self):
        r=self.run_messages([{'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'future'}}],False);self.assertEqual(r[0]['result']['protocolVersion'],'2025-11-25')
    def test_notification_no_response(self):
        r=self.run_messages([{'jsonrpc':'2.0','method':'notifications/cancelled','params':{'requestId':99}}]);self.assertEqual(len(r),1)
    def test_cli_real_subprocess(self):
        root=Path(__file__).resolve().parents[1];env={**os.environ,'PYTHONPATH':str(root/'engine')}
        r=subprocess.run([sys.executable,'-m','hwp_workbench','inspect','form.hwpx','--root',str(self.root)],capture_output=True,text=True,env=env,timeout=10)
        self.assertEqual(r.returncode,0,r.stderr);self.assertEqual(json.loads(r.stdout)['format'],'hwpx');self.assertEqual(r.stderr,'')
    def test_mcp_real_subprocess(self):
        root=Path(__file__).resolve().parents[1];env={**os.environ,'PYTHONPATH':str(root/'engine')}
        messages=[{'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25'}},{'jsonrpc':'2.0','method':'notifications/initialized'},{'jsonrpc':'2.0','id':2,'method':'tools/list'}]
        r=subprocess.run([sys.executable,'-m','hwp_workbench','mcp','--root',str(self.root)],input='\n'.join(map(json.dumps,messages))+'\n',capture_output=True,text=True,env=env,timeout=10)
        self.assertEqual(r.returncode,0,r.stderr);self.assertEqual(len(r.stdout.splitlines()),2);self.assertEqual(r.stderr,'')

    def test_new_tools_cli_mcp_service_parity(self):
        from hwp_workbench.service import invoke

        project = Path(__file__).resolve().parents[1]
        env = {**os.environ, 'PYTHONPATH': str(project / 'engine')}
        (self.root / 'legacy.hwp').write_bytes(make_hwp())
        fields = {'기관명': '새 기관'}
        (self.root / 'fields.json').write_text(json.dumps(fields))
        for path in ['form.hwpx', 'legacy.hwp']:
            doc = Document((self.root / path).read_bytes())
            cases = [
                ('read', {'offset': 0, 'limit': 1}, ['--offset', '0', '--limit', '1']),
                ('search', {'query': '검토'}, ['--query', '검토']),
                ('replace', {'find': '검토', 'replaceWith': '확정'}, ['--find', '검토', '--replace-with', '확정']),
                ('fill', {'fields': fields}, ['--json', 'fields.json']),
                ('set', {'nodeId': doc.nodes[0].id, 'text': doc.nodes[0].text.replace('검토', '확정')},
                 ['--node-id', doc.nodes[0].id, '--text', doc.nodes[0].text.replace('검토', '확정')]),
                ('inspect', {'compact': True}, ['--compact']),
            ]
            for method, options, flags in cases:
                with self.subTest(path=path, method=method):
                    args = {'path': path, **options}
                    expected = invoke(self.root, method, args)
                    replies = self.run_messages([{'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call',
                                                 'params': {'name': 'hwp_' + method, 'arguments': args}}])
                    self.assertFalse(replies[-1]['result']['isError'])
                    self.assertEqual(replies[-1]['result']['structuredContent'], expected)
                    process = subprocess.run([sys.executable, '-m', 'hwp_workbench', method, path,
                                              '--root', str(self.root), *flags],
                                             capture_output=True, text=True, env=env, timeout=10)
                    self.assertEqual(process.returncode, 0, process.stderr)
                    self.assertEqual(json.loads(process.stdout), expected)

    def test_new_tool_schemas_reject_wrong_nested_types(self):
        cases = [
            ('hwp_read', {'limit': True}),
            ('hwp_search', {'query': 'x', 'regex': 'yes'}),
            ('hwp_replace', {'find': 'x', 'replaceWith': 'y', 'nodeIds': [4]}),
            ('hwp_fill', {'fields': {'기관명': 4}}),
            ('hwp_set', {'nodeId': 4, 'text': 'x'}),
        ]
        for name, options in cases:
            with self.subTest(tool=name):
                replies = self.run_messages([{'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call',
                                             'params': {'name': name, 'arguments': {'path': 'form.hwpx', **options}}}])
                self.assertEqual(replies[-1]['error']['code'], -32602)

    def test_new_edit_tool_annotations_and_apply(self):
        replies = self.run_messages([{'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'}])
        tools = {tool['name']: tool for tool in replies[-1]['result']['tools']}
        for name in ['replace', 'fill', 'set']:
            self.assertFalse(tools['hwp_' + name]['annotations']['readOnlyHint'])
            self.assertFalse(tools['hwp_' + name]['annotations']['destructiveHint'])
        args = {'path': 'form.hwpx', 'find': '검토', 'replaceWith': '확정',
                'output': 'replaced.hwpx', 'acknowledgeRisks': True}
        replies = self.run_messages([{'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call',
                                     'params': {'name': 'hwp_replace', 'arguments': args}}])
        self.assertTrue(replies[-1]['result']['structuredContent']['reopenedAndVerified'])

    def test_engine_and_protocol_errors_include_recovery(self):
        replies = self.run_messages([{'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call',
                                     'params': {'name': 'hwp_read', 'arguments': {'path': '../outside.hwpx'}}}])
        error = json.loads(replies[-1]['result']['content'][0]['text'])
        self.assertEqual(error['code'], 'OUTSIDE_WORKSPACE')
        self.assertTrue(error['hint'])
        self.assertIsInstance(error['retryable'], bool)
        self.assertEqual(replies[-1]['result']['structuredContent']['error'], error)
        replies = self.run_messages([{'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call',
                                     'params': {'name': 'hwp_read', 'arguments': {'path': 'form.hwpx', 'limit': 'x'}}}])
        self.assertEqual(replies[-1]['error']['code'], -32602)
        self.assertTrue(replies[-1]['error']['data']['hint'])
        self.assertTrue(replies[-1]['error']['data']['retryable'])

    def test_cli_rpc_and_parser_errors_include_recovery(self):
        project = Path(__file__).resolve().parents[1]
        env = {**os.environ, 'PYTHONPATH': str(project / 'engine')}
        cases = [
            (['read', 'missing.hwpx', '--root', str(self.root)], None, 'READ_FAILED'),
            (['search', 'form.hwpx', '--root', str(self.root)], None, 'BAD_ARGUMENTS'),
            (['fill', 'form.hwpx', '--json', 'bad.json', '--root', str(self.root)], None, 'REQUEST_FAILED'),
            (['rpc', '--root', str(self.root)], json.dumps({'method': 'read', 'args': {'path': 'missing.hwpx'}}), 'READ_FAILED'),
            (['rpc', '--root', str(self.root)], '{bad', 'REQUEST_FAILED'),
        ]
        (self.root / 'bad.json').write_text('{bad')
        for arguments, request, code in cases:
            with self.subTest(arguments=arguments, request=request):
                process = subprocess.run([sys.executable, '-m', 'hwp_workbench', *arguments],
                                         input=request, capture_output=True, text=True, env=env, timeout=10)
                self.assertEqual(process.returncode, 2)
                error = json.loads(process.stderr)['error']
                self.assertEqual(error['code'], code)
                self.assertTrue(error['hint'])
                self.assertIsInstance(error['retryable'], bool)
                self.assertEqual(process.stdout, '')
