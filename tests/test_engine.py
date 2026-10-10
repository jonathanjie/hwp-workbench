import copy,io,json,os,struct,tempfile,unittest,zipfile,zlib
from pathlib import Path
from fixtures import make_hwpx,make_hwp,cfb_from_streams,record
from hwp_workbench.document import Document,HP,sha
from hwp_workbench.cfb import CompoundFile,u32
from hwp_workbench.errors import WorkbenchError
from hwp_workbench.service import invoke
from hwp_workbench.workspace import Workspace

def patch(doc,node=None,text=None):
    n=node or doc.nodes[0]
    return {'schemaVersion':1,'baseSha256':doc.hash,'operations':[{'op':'set_text','nodeId':n.id,'expectedText':n.text,'text':text if text is not None else n.text+' edited'}]}

class EngineTests(unittest.TestCase):
    def expect_code(self,code,fn):
        with self.assertRaises(WorkbenchError) as ctx:fn()
        self.assertEqual(ctx.exception.code,code)
    def test_hwpx_extract_korean(self):
        d=Document(make_hwpx());self.assertEqual(d.format,'hwpx');self.assertIn('마리나체인',[n.text for n in d.nodes])
    def test_hwpx_noop_byte_identical(self):
        b=make_hwpx();self.assertEqual(Document(b).serialize(),b)
    def test_hwpx_variable_length_save_reopen(self):
        d=Document(make_hwpx());d.patch(patch(d,text='확정 검토서 — Final'));b=d.serialize(True)
        self.assertEqual(Document(b).nodes[0].text,'확정 검토서 — Final')
    def test_opaque_parts_preserved(self):
        d=Document(make_hwpx());d.patch(patch(d));other=Document(d.serialize(True))
        for k,v in d.parts.items():
            if k!='Contents/section0.xml':self.assertEqual(other.parts[k],v,k)
    def test_xml_surgical_change(self):
        d=Document(make_hwpx());d.patch(patch(d,text='제목 수정'));other=Document(d.serialize(True))
        self.assertEqual(other.parts['Contents/section0.xml'],d.parts['Contents/section0.xml'].replace(d.nodes[0].text.encode(),'제목 수정'.encode()))
    def test_xml_escapes_and_emoji(self):
        d=Document(make_hwpx());s='<safe> & "quoted" 😀';d.patch(patch(d,text=s));self.assertEqual(Document(d.serialize(True)).nodes[0].text,s)
    def test_empty_selfclosing_node(self):
        d=Document(make_hwpx());n=next(n for n in d.nodes if n.text=='');d.patch(patch(d,n,'채움'));r=Document(d.serialize(True));self.assertEqual(r.by_id[n.id].text,'채움')
    def test_clear_text_node(self):
        d=Document(make_hwpx());d.patch(patch(d,text=''));self.assertEqual(Document(d.serialize(True)).nodes[0].text,'')
    def test_multiple_text_nodes(self):
        d=Document(make_hwpx());p=patch(d);p['operations']+=patch(d,d.nodes[2],'둘째')['operations'];d.patch(p);r=Document(d.serialize(True));self.assertEqual(r.nodes[2].text,'둘째')
    def test_table_cell_target(self):
        d=Document(make_hwpx());n=next(n for n in d.nodes if n.role=='table-cell' and n.text=='마리나체인');d.patch(patch(d,n,'MarinaChain'));self.assertEqual(Document(d.serialize(True)).by_id[n.id].text,'MarinaChain')
    def test_revert_is_noop(self):
        d=Document(make_hwpx());p=patch(d);d.patch(p);p['operations'][0]['expectedText']=p['operations'][0]['text'];p['operations'][0]['text']=d.nodes[0].text;d.patch(p);self.assertEqual(d.serialize(),d.original)
    def test_risk_ack_required(self):
        d=Document(make_hwpx());d.patch(patch(d));self.expect_code('RISK_ACK_REQUIRED',lambda:d.serialize())
    def test_stale_hash(self):
        d=Document(make_hwpx());p=patch(d);p['baseSha256']='wrong';self.expect_code('STALE_DOCUMENT',lambda:d.patch(p))
    def test_stale_expected_text(self):
        d=Document(make_hwpx());p=patch(d);p['operations'][0]['expectedText']='wrong';self.expect_code('STALE_TEXT',lambda:d.patch(p))
    def test_patch_atomicity(self):
        d=Document(make_hwpx());p=patch(d);bad=copy.deepcopy(p['operations'][0]);bad['nodeId']='bad';p['operations'].append(bad);self.expect_code('NODE_NOT_FOUND',lambda:d.patch(p));self.assertEqual(d.edits,{})
    def test_duplicate_patch_target(self):
        d=Document(make_hwpx());p=patch(d);p['operations']*=2;self.expect_code('BAD_PATCH',lambda:d.patch(p))
    def test_control_characters_blocked(self):
        for value in ['one\ntwo','a\tb','\0','\r','\ud800']:
            d=Document(make_hwpx());self.expect_code('CONTROL_TEXT',lambda:d.patch(patch(d,text=value)))
    def test_foreign_zip_rejected(self):
        b=io.BytesIO()
        with zipfile.ZipFile(b,'w') as z:z.writestr('test.txt','hi')
        self.expect_code('NOT_HWPX',lambda:Document(b.getvalue()))
    def test_zip_traversal_rejected(self):
        self.expect_code('UNSAFE_ZIP',lambda:Document(make_hwpx({'../outside':b'evil'})))
    def test_zip_duplicate_rejected(self):
        b=io.BytesIO(make_hwpx())
        import warnings
        with warnings.catch_warnings():
            warnings.simplefilter('ignore')
            with zipfile.ZipFile(b,'a') as z:z.writestr('mimetype','application/hwp+zip')
        self.expect_code('UNSAFE_ZIP',lambda:Document(b.getvalue()))
    def test_dtd_rejected(self):
        xml=f'<!DOCTYPE x [<!ENTITY e "secret">]><hp:t xmlns:hp="{HP}">&e;</hp:t>'
        self.expect_code('UNSAFE_XML',lambda:Document(make_hwpx(section=xml)))
    def test_xml_nested_element_locked(self):
        s=f'<hp:p xmlns:hp="{HP}"><hp:run><hp:t>before<hp:lineBreak/>after</hp:t></hp:run></hp:p>'
        d=Document(make_hwpx(section=s));self.assertFalse(d.nodes[0].editable);self.expect_code('UNSUPPORTED_ELEMENT',lambda:d.patch(patch(d)))
    def test_alternate_namespace_prefix(self):
        s=f'<q:p xmlns:q="{HP}"><q:run><q:t>hello</q:t></q:run></q:p>'
        d=Document(make_hwpx(section=s));d.patch(patch(d,text='수정'));r=Document(d.serialize(True));self.assertEqual(r.nodes[0].text,'수정');self.assertIn(b'<q:t>',r.parts['Contents/section0.xml'])
    def hwpx_default_scripts(self):
        # Hancom 한글 writes these exact UTF-16LE stubs into ordinary HWPX files.
        return {
            'Scripts/headerScripts.js': 'var Documents = XHwpDocuments;var Document = Documents.Active_XHwpDocument;'.encode('utf-16-le'),
            'Scripts/sourceScripts.js': 'function OnDocument_New(){\r\n\t//todo :\r\n}'.encode('utf-16-le'),
        }

    def test_hwpx_default_scripts_editable_and_preserved(self):
        scripts = self.hwpx_default_scripts()
        document = Document(make_hwpx(scripts))
        self.assertFalse(document.block_save)
        self.assertTrue(document.nodes[0].editable)
        warning = next(item for item in document.warnings if item['code'] == 'DEFAULT_SCRIPT_STUB')
        self.assertEqual(warning['severity'], 'info')
        self.assertNotIn('PROTECTED_OR_ACTIVE_CONTENT', [item['code'] for item in document.warnings])
        document.patch(patch(document, text='마리나체인 검토 완'))
        saved = Document(document.serialize(True))
        self.assertEqual(saved.nodes[0].text, '마리나체인 검토 완')
        for name, payload in scripts.items():
            self.assertEqual(saved.parts[name], payload)

    def test_hwpx_nondefault_scripts_block_editing(self):
        def changed_stub(scripts):
            scripts['Scripts/sourceScripts.js'] = 'function OnDocument_New(){ run(); }'.encode('utf-16-le')

        def extra_script(scripts):
            scripts['Scripts/macro.js'] = 'run();'.encode('utf-16-le')

        def missing_stub(scripts):
            del scripts['Scripts/sourceScripts.js']

        for change in [changed_stub, extra_script, missing_stub]:
            with self.subTest(change=change.__name__):
                scripts = self.hwpx_default_scripts()
                change(scripts)
                document = Document(make_hwpx(scripts))
                self.assertTrue(document.block_save)
                self.assertFalse(document.nodes[0].editable)
                self.expect_code('SAVE_BLOCKED', lambda: document.patch(patch(document)))
                self.assertNotIn('DEFAULT_SCRIPT_STUB', [item['code'] for item in document.warnings])

    def test_hwpx_default_scripts_do_not_hide_signatures(self):
        parts = dict(self.hwpx_default_scripts(), **{'META-INF/signatures.xml': b'<signature/>'})
        document = Document(make_hwpx(parts))
        self.assertTrue(document.block_save)
        self.expect_code('SAVE_BLOCKED', lambda: document.patch(patch(document)))

    def test_signature_blocks_editing(self):
        d=Document(make_hwpx({'META-INF/signatures.xml':b'<signature/>'}));self.assertTrue(d.block_save);self.expect_code('SAVE_BLOCKED',lambda:d.patch(patch(d)))
    def test_unknown_binary_is_retained(self):
        d=Document(make_hwpx({'custom/alien.bin':os.urandom(2048)}));d.patch(patch(d));self.assertEqual(Document(d.serialize(True)).parts['custom/alien.bin'],d.parts['custom/alien.bin'])
    def test_hwpx_preview_warning(self):
        codes={w['code'] for w in Document(make_hwpx()).warnings};self.assertTrue({'LAYOUT_UNVERIFIED','PREVIEW_CACHE_RETAINED','TABLE_LAYOUT'}<=codes)
    def test_hwp_read_and_control_skip(self):
        d=Document(make_hwp());self.assertEqual(d.nodes[0].text,'마리나체인 검토 중')
    def test_hwp_sections_in_numeric_order(self):
        parts=dict(CompoundFile(make_hwp(False)).streams)
        parts['BodyText/Section10']=record(67,'tenth'.encode('utf-16le'))
        parts['BodyText/Section2']=record(67,'second'.encode('utf-16le'))
        d=Document(cfb_from_streams(parts))
        self.assertEqual([n.part for n in d.nodes],['BodyText/Section0','BodyText/Section2','BodyText/Section10'])
    def test_hwp_noop_byte_identical(self):
        for compressed in [True,False]:
            b=make_hwp(compressed);self.assertEqual(Document(b).serialize(),b)
    def test_hwp_equal_length_edit_reopen(self):
        for comp in [False,True]:
            d=Document(make_hwp(comp));d.patch(patch(d,text='마리나체인 검토 완'));r=Document(d.serialize(True));self.assertEqual(r.nodes[0].text,'마리나체인 검토 완')
            for part in d.parts:
                if not part.startswith('BodyText/'):self.assertEqual(d.parts[part],r.parts[part])
    def hwp_with_trailer(self, trailer_change=None):
        streams = dict(CompoundFile(make_hwp()).streams)
        for name in ['BodyText/Section0', 'DocInfo']:
            # DocInfo is opaque in the old fixture; use a compressed payload here.
            if name == 'DocInfo':
                compressor = zlib.compressobj(wbits=-15)
                streams[name] = compressor.compress(b'DocInfo test') + compressor.flush()
            payload = zlib.decompress(streams[name], -15)
            trailer = struct.pack('<II', zlib.crc32(payload), len(payload))
            if trailer_change is not None and name == 'BodyText/Section0':
                trailer = trailer_change(trailer)
            streams[name] += trailer
        return cfb_from_streams(streams)

    def test_hwp_valid_compression_trailer_opens(self):
        raw = self.hwp_with_trailer()
        document = Document(raw)
        self.assertEqual(document.nodes[0].text, '마리나체인 검토 중')
        self.assertEqual(document.serialize(), raw)

    def test_hwp_bad_compression_trailer_rejected(self):
        changes = [
            lambda trailer: bytes([trailer[0] ^ 1]) + trailer[1:],
            lambda trailer: trailer[:4] + struct.pack('<I', 1),
            lambda trailer: trailer + b'extra',
            lambda trailer: trailer[:-1],
        ]
        for change in changes:
            with self.subTest(change=change):
                self.expect_code('LIMIT', lambda: Document(self.hwp_with_trailer(change)))

    def test_hwp_compression_trailer_refreshed_after_edit(self):
        document = Document(self.hwp_with_trailer())
        document.patch(patch(document, text='마리나체인 검토 완'))
        saved = document.serialize(True)
        streams = CompoundFile(saved).streams
        decompressor = zlib.decompressobj(-15)
        payload = decompressor.decompress(streams['BodyText/Section0'])
        self.assertEqual(decompressor.unused_data, struct.pack('<II', zlib.crc32(payload), len(payload)))
        self.assertEqual(Document(saved).nodes[0].text, '마리나체인 검토 완')
        self.assertEqual(streams['DocInfo'], document.parts['DocInfo'])

    def test_hwp_length_change_reopens(self):
        document = Document(make_hwp())
        document.patch(patch(document, text='짧음'))
        self.assertEqual(Document(document.serialize(True)).nodes[0].text, '짧음')
    def test_hwp_encrypted_blocked(self):self.expect_code('PROTECTED_HWP',lambda:Document(make_hwp(flags=2)))
    def test_hwp_distribution_blocked(self):self.expect_code('PROTECTED_HWP',lambda:Document(make_hwp(flags=4)))
    def hwp_with_default_scripts(self, change=None, compressed=True):
        streams = dict(CompoundFile(make_hwp(compressed)).streams)
        first = 'var Documents = XHwpDocuments;\r\nvar Document = Documents.Active_XHwpDocument;\r\n'
        second = 'function OnDocument_New()\r\n{\r\n\t//todo : \r\n}\r\n\r\n'
        stub = struct.pack('<I', len(first)) + first.encode('utf-16le')
        stub += struct.pack('<I', len(second)) + second.encode('utf-16le')
        stub += b'\0' * 8 + b'\xff' * 4
        scripts = {
            'Scripts/DefaultJScript': stub,
            'Scripts/JScriptVersion': struct.pack('<Q', 1),
        }
        if change is not None:
            change(scripts)
        for name, payload in scripts.items():
            if compressed:
                compressor = zlib.compressobj(wbits=-15)
                streams[name] = compressor.compress(payload) + compressor.flush()
                streams[name] += struct.pack('<II', zlib.crc32(payload), len(payload))
            else:
                streams[name] = payload
        return cfb_from_streams(streams)

    def test_hwp_default_scripts_editable_and_preserved(self):
        for compressed in [True, False]:
            with self.subTest(compressed=compressed):
                document = Document(self.hwp_with_default_scripts(compressed=compressed))
                self.assertFalse(document.block_save)
                warning = next(item for item in document.warnings if item['code'] == 'DEFAULT_SCRIPT_STUB')
                self.assertEqual(warning['severity'], 'info')
                document.patch(patch(document, text='마리나체인 검토 완'))
                saved = Document(document.serialize(True))
                for name in ['Scripts/DefaultJScript', 'Scripts/JScriptVersion']:
                    self.assertEqual(saved.parts[name], document.parts[name])

    def test_hwp_nondefault_scripts_block_editing(self):
        def changed_stub(scripts):
            name = 'Scripts/DefaultJScript'
            scripts[name] = scripts[name][:-1] + b'\0'

        def changed_version(scripts):
            scripts['Scripts/JScriptVersion'] = struct.pack('<Q', 2)

        def extra_script(scripts):
            scripts['Scripts/Extra'] = b'extra'

        def missing_version(scripts):
            del scripts['Scripts/JScriptVersion']

        for change in [changed_stub, changed_version, extra_script, missing_version]:
            with self.subTest(change=change):
                document = Document(self.hwp_with_default_scripts(change))
                self.assertTrue(document.block_save)
                self.assertFalse(document.nodes[0].editable)
                self.expect_code('SAVE_BLOCKED', lambda: document.patch(patch(document)))
                self.assertNotIn('DEFAULT_SCRIPT_STUB', [item['code'] for item in document.warnings])

    def test_hwp_signature_blocked(self):
        d=Document(make_hwp(flags=64));self.assertTrue(d.block_save)
    def test_cfb_v4(self):
        d=Document(make_hwp(major=4));d.patch(patch(d,text='마리나체인 검토 완'));r=Document(d.serialize(True));self.assertEqual(r.nodes[0].text,'마리나체인 검토 완')
    def test_cfb_long_extended_record(self):
        d=Document(make_hwp(False,long=True));n=d.nodes[0];d.patch(patch(d,text=n.text.replace('테스트','검토서')));self.assertTrue(Document(d.serialize(True)).nodes[0].text.startswith('검토서'))
    def test_cfb_fat_cycles_rejected(self):
        b=bytearray(make_hwp());ss=512;fatid=u32(b,76);directoryid=u32(b,48);struct.pack_into('<I',b,(fatid+1)*ss+directoryid*4,directoryid)
        self.expect_code('BAD_CFB',lambda:CompoundFile(bytes(b)))
    def test_cfb_opaque_stream_resize(self):
        c=CompoundFile(make_hwp());new=b'large'*2000;b=c.rewrite({'DocInfo':new});r=CompoundFile(b);self.assertEqual(r.streams['DocInfo'],new)
    def test_cfb_ministream_transition(self):
        c=CompoundFile(make_hwp());large=b'a'*4097;r=CompoundFile(c.rewrite({'DocInfo':large}));r=CompoundFile(r.rewrite({'DocInfo':b'tiny'}));self.assertEqual(r.streams['DocInfo'],b'tiny')
    def test_cfb_difat(self):
        b=cfb_from_streams({'Large':b'x'*8_000_000,'Small':b'y'});c=CompoundFile(b);self.assertGreater(u32(b,44),109);self.assertEqual(c.streams['Small'],b'y');self.assertEqual(c.streams['Large'],b'x'*8_000_000)
    def test_cfb_truncated(self):self.expect_code('BAD_CFB',lambda:CompoundFile(make_hwp()[:-1]))
    def test_foreign_bytes_rejected(self):self.expect_code('UNSUPPORTED_FORMAT',lambda:Document(b'not a document'))

class WorkspaceTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name);self.ws=Workspace(self.root);(self.root/'test.hwpx').write_bytes(make_hwpx())
    def tearDown(self):self.temp.cleanup()
    def test_read(self):self.assertTrue(self.ws.read('test.hwpx').startswith(b'PK'))
    def test_outside_root(self):
        with self.assertRaises(WorkbenchError):self.ws.read('../test.hwpx')
    def test_symlink_file(self):
        (self.root/'link').symlink_to(self.root/'test.hwpx')
        with self.assertRaises(WorkbenchError):self.ws.read('link')
    def test_symlink_directory(self):
        (self.root/'sub').mkdir();(self.root/'link').symlink_to(self.root/'sub',target_is_directory=True)
        with self.assertRaises(WorkbenchError):self.ws.write_new('link/evil',b'test')
    def test_write_exclusive(self):
        self.ws.write_new('new.txt',b'first')
        with self.assertRaises(WorkbenchError):self.ws.write_new('new.txt',b'second')
        self.assertEqual((self.root/'new.txt').read_bytes(),b'first')
    def test_private_output_mode(self):
        self.ws.write_new('new.txt',b'x');self.assertEqual((self.root/'new.txt').stat().st_mode&0o777,0o600)
    def test_apply_preserves_source(self):
        original=(self.root/'test.hwpx').read_bytes();d=Document(original);p=patch(d)
        result=invoke(self.root,'apply',{'path':'test.hwpx','patch':p,'output':'edited.hwpx','acknowledgeRisks':True})
        self.assertTrue(result['reopenedAndVerified']);self.assertEqual((self.root/'test.hwpx').read_bytes(),original)
    def test_apply_no_overwrite(self):
        d=Document(self.ws.read('test.hwpx'))
        with self.assertRaises(WorkbenchError):invoke(self.root,'apply',{'path':'test.hwpx','patch':patch(d),'output':'test.hwpx','acknowledgeRisks':True})
    def test_apply_requires_ack(self):
        d=Document(self.ws.read('test.hwpx'))
        with self.assertRaises(WorkbenchError):invoke(self.root,'apply',{'path':'test.hwpx','patch':patch(d),'output':'new.hwpx'})
        self.assertFalse((self.root/'new.hwpx').exists())
    def test_plan_no_output(self):
        d=Document(self.ws.read('test.hwpx'));result=invoke(self.root,'plan',{'path':'test.hwpx','patch':patch(d)});self.assertEqual(len(result['changes']),1);self.assertEqual(len(list(self.root.iterdir())),1)
    def test_output_format_enforced(self):
        d=Document(self.ws.read('test.hwpx'))
        with self.assertRaises(WorkbenchError):invoke(self.root,'apply',{'path':'test.hwpx','patch':patch(d),'output':'fake.hwp','acknowledgeRisks':True})
