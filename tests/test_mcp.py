import io,json,tempfile,unittest,subprocess,os,sys
from pathlib import Path
from fixtures import make_hwpx
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
        r=self.run_messages([{'jsonrpc':'2.0','id':2,'method':'tools/list'}]);self.assertEqual(r[0]['result']['protocolVersion'],'2025-11-25');self.assertEqual(len(r[1]['result']['tools']),5)
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
