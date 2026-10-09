"""Explicit conversion tests use only self-authored, in-memory packages."""
import os
import unittest

from convert_fixtures import docx, png


class ForwardMapperTests(unittest.TestCase):
    def test_docx_reader_preserves_rich_properties(self):
        from hwp_workbench.convert.docx_read import read_docx
        from hwp_workbench.convert.model import Table
        rich = read_docx(docx())
        self.assertEqual(rich.paragraph_texts(), ['Heading 한글', 'Styled text \nsecond line', 'Bullet item', 'Number item', 'Across columns', 'Across rows', 'Right one', 'Right two', 'Image caption', 'New page', ''])
        heading, styled, bullet, numbered = rich.sections[0].blocks[:4]
        self.assertEqual(heading.heading, 1)
        self.assertEqual(heading.runs[0].style['size'], 18)
        self.assertEqual(styled.runs[0].style, dict(bold=True, italic=True, underline='single', strike=True, size=14, colour='2468AC', font='Arial'))
        self.assertEqual(styled.style['left'], 360)
        self.assertEqual(styled.style['line'], 300)
        self.assertEqual(bullet.numbering['kind'], 'bullet')
        self.assertEqual(numbered.numbering['start'], 3)
        table = rich.sections[0].blocks[4]
        self.assertIsInstance(table, Table)
        self.assertEqual(table.widths, [1800, 2200])
        self.assertEqual(table.rows[0][0].colspan, 2)
        self.assertEqual(table.rows[1][0].rowspan, 2)
        self.assertEqual(table.rows[0][0].shading, 'DDEEFF')
        self.assertEqual(table.rows[0][0].borders['top']['colour'], '123456')
        self.assertEqual(rich.sections[0].blocks[5].runs[-1].image.data, png())
        self.assertEqual(rich.sections[0].page['width'], 16838)
        self.assertTrue(rich.sections[0].blocks[6].page_break)
        self.assertEqual(rich.unsupported, {})

    def test_hwpx_writer_reopens_with_native_text(self):
        from hwp_workbench.convert.docx_read import read_docx
        from hwp_workbench.convert.hwpx_write import write_hwpx
        from hwp_workbench.document import Document
        rich = read_docx(docx())
        native = Document(write_hwpx(rich))
        paragraphs = {}
        for node in native.nodes:
            paragraphs.setdefault((node.part, node.paragraph), []).append(node.text)
        self.assertEqual([''.join(texts) for texts in paragraphs.values()], rich.paragraph_texts())
        self.assertIn(png(), native.parts.values())

    def test_hwpx_opens_and_renders_in_rhwp(self):
        from hwp_workbench.convert.docx_read import read_docx
        from hwp_workbench.convert.hwpx_write import write_hwpx
        from hwp_workbench import independent
        payload = write_hwpx(read_docx(docx()))
        observed = independent.probe(payload)
        if observed.get('available') is False:
            self.skipTest(observed['reason'])
        self.assertGreaterEqual(observed['pageCount'], 1)
        for text in ['Heading 한글', 'Across columns', 'Across rows', 'Image caption']:
            self.assertIn(text, observed['fullText'])
        rendered = independent.invoke(payload, 'render', page=0)
        self.assertIn('<svg', rendered['svg'])
        from xml.etree import ElementTree as ET
        svg = ET.fromstring(rendered['svg'])
        self.assertIn('Heading', ''.join(node.text or '' for node in svg.iter('{http://www.w3.org/2000/svg}text')))
        self.assertIn('data:image/png;base64,', rendered['svg'])


class ReverseMapperTests(unittest.TestCase):
    def test_rich_hwpx_to_docx_preserves_content_and_geometry(self):
        from hwp_workbench.convert.docx_read import read_docx
        from hwp_workbench.convert.hwpx_write import write_hwpx
        from hwp_workbench.convert.hwpx_read_rich import read_hwpx
        from hwp_workbench.convert.docx_write import write_docx
        source = read_docx(docx())
        pivot = read_hwpx(write_hwpx(source))
        reopened = read_docx(write_docx(pivot))
        self.assertEqual(reopened.paragraph_texts(), source.paragraph_texts())
        self.assertEqual(pivot.paragraph_texts(), source.paragraph_texts())
        for rich in [pivot, reopened]:
            self.assertEqual(rich.sections[0].page, source.sections[0].page)
            self.assertEqual(rich.sections[0].blocks[0].heading, 1)
            self.assertTrue(rich.sections[0].blocks[1].runs[0].style['strike'])
            self.assertEqual(rich.sections[0].blocks[1].style['before'], 120)
            table = rich.sections[0].blocks[4]
            self.assertEqual(table.widths, [1800, 2200])
            self.assertEqual(table.rows[0][0].colspan, 2)
            self.assertEqual(table.rows[1][0].rowspan, 2)
            self.assertEqual(table.rows[0][0].shading, 'DDEEFF')
            self.assertEqual(table.rows[0][0].borders['top']['size'], 8)
            self.assertEqual(table.rows[1][0].borders['bottom']['kind'], 'double')
            self.assertEqual(rich.sections[0].blocks[5].runs[-1].image.data, png())
            self.assertEqual(rich.sections[0].blocks[2].numbering['kind'], 'bullet')
            self.assertEqual(rich.sections[0].blocks[3].numbering['start'], 3)
            self.assertTrue(rich.sections[0].blocks[6].page_break)
            self.assertEqual(rich.unsupported, {})

    def test_docx_package_parts_use_unprefixed_namespaces(self):
        # Regression: ElementTree wrote `ns0:Types` and `ns0:Relationships`;
        # LibreOffice then refused the whole DOCX as "could not be loaded".
        import io
        import zipfile
        from hwp_workbench.convert.docx_read import read_docx
        from hwp_workbench.convert.docx_write import write_docx
        package = zipfile.ZipFile(io.BytesIO(write_docx(read_docx(docx()))))
        expected = {'[Content_Types].xml': b'<Types xmlns="', '_rels/.rels': b'<Relationships xmlns="',
                    'word/_rels/document.xml.rels': b'<Relationships xmlns="'}
        for name, start in expected.items():
            with self.subTest(part=name):
                self.assertIn(start, package.read(name))
                self.assertNotIn(b'ns0:', package.read(name))

    def test_libreoffice_independently_opens_output_docx(self):
        if os.environ.get('HWP_WORKBENCH_LIBREOFFICE_TESTS') != '1':
            self.skipTest('real LibreOffice checks are opt-in: set HWP_WORKBENCH_LIBREOFFICE_TESTS=1')
        from hwp_workbench.convert.docx_read import read_docx
        from hwp_workbench.convert.docx_write import write_docx
        from hwp_workbench.convert import libreoffice
        if not libreoffice.executable():
            self.skipTest('LibreOffice is not installed; independent DOCX-to-PDF check did not run.')
        payload = write_docx(read_docx(docx()))
        pdf, report = libreoffice.convert(payload, 'docx', 'pdf')
        self.assertTrue(report['available'])
        self.assertTrue(pdf.startswith(b'%PDF-'))


class ConversionWorkspace(unittest.TestCase):
    def setUp(self):
        import tempfile
        from pathlib import Path
        self.temporary = tempfile.TemporaryDirectory(prefix='test-hwp-convert-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / 'source.docx').write_bytes(docx())


class ConversionServiceTests(ConversionWorkspace):
    def test_dry_run_and_acknowledged_publication(self):
        from hwp_workbench.service import invoke
        from hwp_workbench.document import Document
        for arguments in [{}, {'output': 'copy.hwpx'}, {'acknowledgeRisks': True}]:
            report = invoke(self.root, 'convert', {'path': 'source.docx', 'to': 'hwpx'} | arguments)
            self.assertEqual(report['from'], 'docx')
            self.assertEqual(report['to'], 'hwpx')
            self.assertEqual(report['route'], ['docx → hwpx (Python direct mapper)'])
            self.assertEqual(report['unsupported'], {})
            self.assertEqual(report['textCheck']['missingParagraphs'], [])
            self.assertEqual(report['textCheck']['changedParagraphs'], [])
            self.assertNotIn('output', report)
            self.assertFalse((self.root / 'copy.hwpx').exists())
        report = invoke(self.root, 'convert', {'path': 'source.docx', 'to': 'hwpx', 'output': 'copy.hwpx', 'acknowledgeRisks': True})
        self.assertEqual(Document((self.root / 'copy.hwpx').read_bytes()).hash, report['outputSha256'])
        self.assertEqual((self.root / 'source.docx').read_bytes(), docx())

    def test_native_routes_and_docx_output(self):
        from hwp_workbench.service import invoke
        from hwp_workbench.convert.docx_read import read_docx
        from hwp_workbench.convert.hwpx_write import write_hwpx
        (self.root / 'source.hwpx').write_bytes(write_hwpx(read_docx(docx())))
        result = invoke(self.root, 'convert', dict(path='source.hwpx', to='docx', output='reverse.docx', acknowledgeRisks=True))
        self.assertEqual(result['textCheck']['missingParagraphs'], [])
        self.assertEqual(read_docx((self.root / 'reverse.docx').read_bytes()).paragraph_texts(), read_docx(docx()).paragraph_texts())
        result = invoke(self.root, 'convert', dict(path='source.hwpx', to='hwp', output='native.hwp', acknowledgeRisks=True))
        if result.get('available') is False:
            self.skipTest(result['engineReports'][-1]['reason'])
        self.assertIn('lossReport', result['engineReports'][0])
        for source, target in [('native.hwp', 'hwpx'), ('native.hwp', 'docx'), ('source.docx', 'hwp')]:
            with self.subTest(source=source, target=target):
                report = invoke(self.root, 'convert', {'path': source, 'to': target})
                self.assertTrue(report['textCheck']['performed'])
                self.assertTrue(report['route'])
                self.assertEqual(report['textCheck']['missingParagraphs'], [])

    def test_cli_and_mcp_share_conversion_contract(self):
        import io
        import json
        import os
        import subprocess
        from pathlib import Path
        from hwp_workbench.mcp import serve, TOOLS
        tool = next(item for item in TOOLS if item['name'] == 'hwp_convert')
        self.assertFalse(tool['annotations']['readOnlyHint'])
        project = Path(__file__).resolve().parents[1]
        command = [os.sys.executable, '-m', 'hwp_workbench', 'convert', 'source.docx', '--to', 'hwpx', '--root', str(self.root)]
        env = dict(os.environ, PYTHONPATH=str(project / 'engine'))
        cli = subprocess.run(command, env=env, capture_output=True, text=True, check=False)
        self.assertEqual(cli.returncode, 0, cli.stderr)
        stream = io.StringIO('\n'.join(json.dumps(message) for message in [
            dict(jsonrpc='2.0', id=1, method='initialize', params={}),
            dict(jsonrpc='2.0', method='notifications/initialized'),
            dict(jsonrpc='2.0', id=2, method='tools/call', params={'name': 'hwp_convert', 'arguments': {'path': 'source.docx', 'to': 'hwpx'}}),
        ]) + '\n')
        output = io.StringIO()
        serve(self.root, stream, output)
        mcp = json.loads(output.getvalue().splitlines()[-1])['result']
        self.assertFalse(mcp['isError'])
        self.assertEqual(json.loads(cli.stdout), mcp['structuredContent'])


class ConversionSafetyTests(ConversionWorkspace):

    def assert_code(self, code, call):
        from hwp_workbench.errors import WorkbenchError
        with self.assertRaises(WorkbenchError) as context:
            call()
        self.assertEqual(context.exception.code, code)

    def test_no_overwrite_confinement_and_source_race(self):
        from unittest.mock import patch
        from hwp_workbench.service import invoke
        from hwp_workbench.workspace import Workspace
        arguments = dict(path='source.docx', to='hwpx', output='copy.hwpx', acknowledgeRisks=True)
        (self.root / 'copy.hwpx').write_bytes(b'KEEP')
        self.assert_code('OUTPUT_EXISTS', lambda: invoke(self.root, 'convert', arguments))
        self.assertEqual((self.root / 'copy.hwpx').read_bytes(), b'KEEP')
        self.assert_code('OUTSIDE_WORKSPACE', lambda: invoke(self.root, 'convert', arguments | {'output': '../outside.hwpx'}))
        self.assert_code('OUTSIDE_WORKSPACE', lambda: invoke(self.root, 'convert', arguments | {'path': '../outside.docx'}))
        (self.root / 'linked').symlink_to(self.root, target_is_directory=True)
        self.assert_code('UNSAFE_PATH', lambda: invoke(self.root, 'convert', arguments | {'output': 'linked/new.hwpx'}))
        self.assert_code('OUTPUT_FORMAT', lambda: invoke(self.root, 'convert', arguments | {'output': 'wrong.docx'}))
        original_read = Workspace.read
        calls = 0
        def raced_read(workspace, path, **kwargs):
            nonlocal calls
            data = original_read(workspace, path, **kwargs)
            calls += 1
            return data if calls == 1 else data + b'changed'
        with patch.object(Workspace, 'read', raced_read):
            self.assert_code('STALE_DOCUMENT', lambda: invoke(self.root, 'convert', arguments | {'output': 'race.hwpx'}))
        self.assertFalse((self.root / 'race.hwpx').exists())

    def test_injected_rhwp_loss_is_preserved_in_report(self):
        import base64
        from unittest.mock import patch
        from hwp_workbench.service import invoke
        from fixtures import make_hwp
        payload = make_hwp()
        export = {'bytesBase64': base64.b64encode(payload).decode(), 'lossReport': {'losses': [{'feature': 'equation', 'count': 2, 'message': 'Injected loss'}]}}
        with patch('hwp_workbench.convert.operation.independent.invoke', return_value=export):
            report = invoke(self.root, 'convert', dict(path='source.docx', to='hwp'))
        self.assertEqual(report['engineReports'][-1]['lossReport'], export['lossReport'])
        self.assertFalse(report['textCheck']['matches'])
        self.assertTrue(report['textCheck']['missingParagraphs'] or report['textCheck']['changedParagraphs'])
        self.assertFalse((self.root / 'source.hwp').exists())

    def test_unsupported_objects_are_counted_in_both_directions(self):
        from hwp_workbench.convert.docx_read import read_docx, W
        from hwp_workbench.service import invoke
        from fixtures import make_hwpx, hwpx_paragraph, HP, HC
        extra = ('<w:p><w:fldSimple w:instr="DATE"><w:r><w:t>Field value</w:t></w:r></w:fldSimple>'
                 '<w:ins w:id="1"><w:r><w:t>Inserted text</w:t></w:r></w:ins><w:del w:id="2"><w:r><w:delText>Deleted text</w:delText></w:r></w:del>'
                 '<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:r><m:t>x</m:t></m:r></m:oMath>'
                 '<w:r><w:pict><w:txbxContent><w:p><w:r><w:t>Box text</w:t></w:r></w:p></w:txbxContent></w:pict></w:r></w:p>')
        parts = {'word/header1.xml': f'<w:hdr xmlns:w="{W}"><w:p/></w:hdr>',
                 'word/footer1.xml': f'<w:ftr xmlns:w="{W}"><w:p/></w:ftr>',
                 'word/footnotes.xml': f'<w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:p/></w:footnote></w:footnotes>',
                 'word/comments.xml': f'<w:comments xmlns:w="{W}"><w:comment w:id="1"/></w:comments>'}
        payload = docx(extra, parts)
        rich = read_docx(payload)
        for feature, count in {'fields': 1, 'trackedChanges': 2, 'equations': 1, 'headers': 1, 'footers': 1, 'footnotes': 1, 'comments': 1, 'textBoxes': 1, 'shapes': 1}.items():
            self.assertEqual(rich.unsupported.get(feature), count, feature)
        (self.root / 'unsupported.docx').write_bytes(payload)
        report = invoke(self.root, 'convert', dict(path='unsupported.docx', to='hwpx'))
        self.assertEqual(report['unsupported'], rich.unsupported)
        section = (f'<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="{HP}" xmlns:hc="{HC}">'
                   + hwpx_paragraph('Visible', prefix='<hp:ctrl><hp:header><hp:subList>' + hwpx_paragraph('Header lost') + '</hp:subList></hp:header><hp:fieldBegin/></hp:ctrl><hp:equation/>') + '</hs:sec>')
        (self.root / 'unsupported.hwpx').write_bytes(make_hwpx(section=section))
        report = invoke(self.root, 'convert', dict(path='unsupported.hwpx', to='docx'))
        self.assertEqual(report['unsupported']['headers'], 1)
        self.assertEqual(report['unsupported']['fields'], 1)
        self.assertEqual(report['unsupported']['equations'], 1)
        self.assertTrue(report['textCheck']['missingParagraphCount'] or report['textCheck']['changedParagraphCount'])

    def test_optional_dependencies_missing_are_reported_without_writing(self):
        from unittest.mock import patch
        from hwp_workbench.service import invoke
        (self.root / 'legacy.rtf').write_bytes(b'{\\rtf1 Hello}')
        with patch('hwp_workbench.convert.libreoffice.executable', return_value=None):
            report = invoke(self.root, 'convert', dict(path='legacy.rtf', to='docx', output='new.docx', acknowledgeRisks=True))
        self.assertFalse(report['available'])
        self.assertFalse(report['engineReports'][0]['available'])
        self.assertFalse(report['textCheck']['performed'])
        self.assertFalse((self.root / 'new.docx').exists())
        with patch('hwp_workbench.convert.operation.independent.invoke', return_value={'available': False, 'reason': 'Test runtime absent'}):
            report = invoke(self.root, 'convert', dict(path='source.docx', to='hwp'))
        self.assertFalse(report['available'])
        self.assertEqual(report['engineReports'][-1]['reason'], 'Test runtime absent')

    def test_same_format_copy_keeps_original_bytes(self):
        from hwp_workbench.service import invoke
        report = invoke(self.root, 'convert', dict(path='source.docx', to='docx', output='same.docx', acknowledgeRisks=True))
        self.assertEqual((self.root / 'same.docx').read_bytes(), (self.root / 'source.docx').read_bytes())
        self.assertTrue(report['textCheck']['matches'])


class MalformedDocxTests(unittest.TestCase):
    def assert_rejected(self, payload, code):
        from hwp_workbench.convert.docx_read import read_docx
        from hwp_workbench.errors import WorkbenchError
        with self.assertRaises(WorkbenchError) as context:
            read_docx(payload)
        self.assertEqual(context.exception.code, code)

    def test_dtd_and_xml_depth_and_member_bounds(self):
        from convert_fixtures import package
        from unittest.mock import patch
        from hwp_workbench.convert import package as bounded_package
        self.assert_rejected(docx(extra_parts={'word/evil.xml': '<!DOCTYPE a [<!ENTITY x "boom">]><a>&x;</a>'}), 'UNSAFE_XML')
        self.assert_rejected(docx(extra_parts={'word/evil.xml': '<a>' * 130 + '</a>' * 130}), 'LIMIT')
        self.assert_rejected(docx(extra_parts={'../escape': b'no'}), 'UNSAFE_ZIP')
        with patch.object(bounded_package, 'MAX_PART', 32):
            self.assert_rejected(docx(), 'LIMIT')
        with patch.object(bounded_package, 'MAX_EXPANDED', 100):
            self.assert_rejected(docx(), 'LIMIT')
        self.assert_rejected(package({f'part{i}': b'x' for i in range(2049)}), 'LIMIT')
        self.assert_rejected(package({'word/document.xml': '<wrong/>'}), 'NOT_DOCX')

    def test_duplicate_archive_members_and_invalid_utf8(self):
        import io
        import warnings
        import zipfile
        output = io.BytesIO()
        with warnings.catch_warnings():
            warnings.simplefilter('ignore')
            with zipfile.ZipFile(output, 'w') as archive:
                archive.writestr('same', b'a')
                archive.writestr('same', b'b')
        self.assert_rejected(output.getvalue(), 'UNSAFE_ZIP')
        self.assert_rejected(docx(extra_parts={'word/evil.xml': b'<a>\xff</a>'}), 'XML_ENCODING')

    def test_oversized_spans_and_cyclic_styles_fail_closed(self):
        from convert_fixtures import W
        self.assert_rejected(docx('<w:tbl><w:tr><w:tc><w:tcPr><w:gridSpan w:val="1001"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>'), 'LIMIT')
        styles = f'<w:styles xmlns:w="{W}"><w:style w:styleId="Heading1"><w:basedOn w:val="Heading1"/></w:style></w:styles>'
        self.assert_rejected(docx(extra_parts={'word/styles.xml': styles}), 'BAD_XML')


class LibreOfficeSafetyTests(unittest.TestCase):
    def test_timeout_uses_private_copies_fresh_profile_and_cleans_up(self):
        from pathlib import Path
        import subprocess
        from unittest.mock import patch
        from hwp_workbench.convert import libreoffice
        from hwp_workbench.errors import WorkbenchError
        captured = []
        def timed_out(command, **options):
            captured.extend(command)
            self.assertEqual(options['timeout'], 60)
            self.assertFalse(options['shell'])
            self.assertIn('--norestore', command)
            input_path = Path(command[-1])
            self.assertEqual(input_path.read_bytes(), b'{\\rtf1 Hello}')
            self.assertEqual(input_path.parent.stat().st_mode & 0o777, 0o700)
            profile = input_path.parent / 'profile/user/registrymodifications.xcu'
            self.assertIn('MacroSecurityLevel', profile.read_text())
            raise subprocess.TimeoutExpired(command, 60)
        with patch.object(libreoffice, 'executable', return_value='/mock/soffice'), patch.object(libreoffice.subprocess, 'run', side_effect=timed_out):
            with self.assertRaises(WorkbenchError) as context:
                libreoffice.convert(b'{\\rtf1 Hello}', 'rtf', 'docx')
        self.assertEqual(context.exception.code, 'LIBREOFFICE_TIMEOUT')
        self.assertFalse(Path(captured[-1]).parent.exists())

    def test_legacy_routes_use_bounded_docx_intermediate(self):
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        from hwp_workbench.service import invoke
        from convert_fixtures import package
        from fixtures import cfb_from_streams
        sources = {'doc': cfb_from_streams({'WordDocument': b'authored placeholder'}),
                   'odt': package({'mimetype': b'application/vnd.oasis.opendocument.text', 'content.xml': '<document/>'}),
                   'rtf': b'{\\rtf1 Hello}'}
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for source, payload in sources.items():
                (root / ('input.' + source)).write_bytes(payload)
                for target in ['hwp', 'hwpx', 'docx']:
                    with self.subTest(source=source, target=target), patch('hwp_workbench.convert.libreoffice.convert', return_value=(docx(), {'available': True, 'engine': 'LibreOffice'})):
                        report = invoke(root, 'convert', dict(path='input.' + source, to=target))
                    self.assertEqual(report['route'][0], f'{source} → docx (LibreOffice)')
                    self.assertEqual(report['unsupported'].get('legacyImportUnverified'), 1)
                    if report['available']:
                        self.assertIn('intermediate', report['textCheck']['source'])
                    else:
                        self.assertFalse(report['textCheck']['performed'])

    def test_real_rtf_import_when_runtime_can_execute(self):
        if os.environ.get('HWP_WORKBENCH_LIBREOFFICE_TESTS') != '1':
            self.skipTest('real LibreOffice checks are opt-in: set HWP_WORKBENCH_LIBREOFFICE_TESTS=1')
        from hwp_workbench.convert import libreoffice
        from hwp_workbench.convert.docx_read import read_docx
        if not libreoffice.executable():
            self.skipTest('LibreOffice is not installed; real RTF import did not run.')
        converted, _ = libreoffice.convert(b'{\\rtf1\\ansi Authored RTF text\\par Second paragraph}', 'rtf', 'docx')
        self.assertEqual(read_docx(converted).paragraph_texts(), ['Authored RTF text', 'Second paragraph'])


class AdditionalMappingTests(unittest.TestCase):
    def test_inline_page_break_keeps_text_order_and_break_location(self):
        from hwp_workbench.convert.docx_read import read_docx
        from hwp_workbench.convert.hwpx_write import write_hwpx
        from hwp_workbench.convert.hwpx_read_rich import read_hwpx
        from hwp_workbench.convert.docx_write import write_docx
        source = read_docx(docx('<w:p><w:r><w:t>Before break</w:t><w:br w:type="page"/><w:t>After break</w:t></w:r></w:p>'))
        self.assertEqual(source.paragraph_texts()[-2:], ['Before break', 'After break'])
        self.assertFalse(source.sections[0].blocks[-2].page_break)
        self.assertTrue(source.sections[0].blocks[-1].page_break)
        pivot = read_hwpx(write_hwpx(source))
        self.assertEqual(pivot.paragraph_texts(), source.paragraph_texts())
        self.assertTrue(pivot.sections[0].blocks[-1].page_break)
        self.assertTrue(read_docx(write_docx(pivot)).sections[0].blocks[-1].page_break)

    def test_character_style_does_not_reset_paragraph_style_size(self):
        from hwp_workbench.convert.docx_read import read_docx
        from convert_fixtures import W
        styles = (f'<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:styleId="Heading1"><w:name w:val="Heading 1"/><w:rPr><w:sz w:val="36"/></w:rPr></w:style>'
                  '<w:style w:type="character" w:styleId="Emphasis"><w:rPr><w:i/></w:rPr></w:style></w:styles>')
        rich = read_docx(docx('<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:rPr><w:rStyle w:val="Emphasis"/></w:rPr><w:t>Emphasis heading</w:t></w:r></w:p>', {'word/styles.xml': styles}))
        self.assertEqual(rich.sections[0].blocks[-1].runs[0].style['size'], 18)
        self.assertTrue(rich.sections[0].blocks[-1].runs[0].style['italic'])

    def test_table_outer_and_inside_borders_are_resolved_per_cell(self):
        from hwp_workbench.convert.docx_read import read_docx
        body = ('<w:tbl><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="8" w:color="FF0000"/>'
                '<w:insideH w:val="single" w:sz="4" w:color="00FF00"/></w:tblBorders></w:tblPr>'
                '<w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Top</w:t></w:r></w:p></w:tc></w:tr>'
                '<w:tr><w:tc><w:p><w:r><w:t>Bottom</w:t></w:r></w:p></w:tc></w:tr></w:tbl>')
        table = read_docx(docx(body)).sections[0].blocks[-1]
        self.assertEqual(table.rows[0][0].borders['top']['colour'], 'FF0000')
        self.assertEqual(table.rows[0][0].borders['bottom']['colour'], '00FF00')
        self.assertEqual(table.rows[1][0].borders['top']['colour'], '00FF00')

    def test_text_checks_report_duplicate_deletion_and_reordering(self):
        from hwp_workbench.convert.operation import text_check
        report = text_check(['Repeated', 'Repeated', 'Last'], ['Repeated', 'Last'])
        self.assertEqual(report['missingParagraphCount'], 1)
        self.assertFalse(report['matches'])
        self.assertFalse(text_check(['First', 'Second'], ['Second', 'First'])['matches'])
        self.assertTrue(text_check(['한글\n  text'], ['한글 text'])['matches'])


class LibreOfficeOptInTests(unittest.TestCase):
    def test_unset_opt_in_never_reaches_executable(self):
        from unittest.mock import patch
        with patch.dict(os.environ, {'HWP_WORKBENCH_LIBREOFFICE_TESTS': '0'}), patch('hwp_workbench.convert.libreoffice.executable', side_effect=AssertionError('Real process must not be reached')):
            for case in [ReverseMapperTests('test_libreoffice_independently_opens_output_docx'),
                         LibreOfficeSafetyTests('test_real_rtf_import_when_runtime_can_execute')]:
                result = unittest.TestResult()
                case.run(result)
                self.assertEqual(len(result.skipped), 1)
                self.assertEqual(result.skipped[0][1], 'real LibreOffice checks are opt-in: set HWP_WORKBENCH_LIBREOFFICE_TESTS=1')
                self.assertEqual(result.errors, [])


class ReportHonestyTests(unittest.TestCase):
    def test_complex_field_counts_once_and_revisions_are_named(self):
        from hwp_workbench.convert.docx_read import read_docx
        body = ('<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>DATE</w:instrText></w:r>'
                '<w:r><w:fldChar w:fldCharType="separate"/><w:t>Today</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>'
                '<w:r><w:rPr><w:rPrChange w:id="1"><w:rPr/></w:rPrChange></w:rPr><w:t>Revised style</w:t></w:r></w:p>')
        report = read_docx(docx(body)).unsupported
        self.assertEqual(report['fields'], 1)
        self.assertEqual(report['trackedChanges'], 1)

    def test_image_crop_is_reported(self):
        import io
        import zipfile
        from convert_fixtures import package
        from hwp_workbench.convert.docx_read import read_docx
        with zipfile.ZipFile(io.BytesIO(docx())) as archive:
            parts = {name: archive.read(name) for name in archive.namelist()}
        text = parts['word/document.xml'].decode().replace('</pic:blipFill>', '<a:srcRect l="1000"/></pic:blipFill>')
        parts['word/document.xml'] = text.encode()
        self.assertEqual(read_docx(package(parts)).unsupported['imageCropping'], 1)


class ReviewRegressionTests(unittest.TestCase):
    def test_default_paragraph_style_applies_without_explicit_reference(self):
        from hwp_workbench.convert.docx_read import read_docx
        from convert_fixtures import W
        styles = (f'<w:styles xmlns:w="{W}"><w:style w:type="paragraph" w:default="1" w:styleId="Normal">'
                  '<w:pPr><w:jc w:val="right"/></w:pPr><w:rPr><w:b/><w:sz w:val="36"/><w:rFonts w:ascii="Times New Roman"/></w:rPr>'
                  '</w:style></w:styles>')
        rich = read_docx(docx(extra_parts={'word/styles.xml': styles}))
        paragraph = rich.sections[0].blocks[5]
        self.assertTrue(paragraph.runs[0].style['bold'])
        self.assertEqual(paragraph.runs[0].style['size'], 18)
        self.assertEqual(paragraph.runs[0].style['font'], 'Times New Roman')
        self.assertEqual(paragraph.style['alignment'], 'right')

    def test_explicit_num_id_zero_disables_style_list(self):
        from hwp_workbench.convert.docx_read import read_docx
        from convert_fixtures import W
        styles = (f'<w:styles xmlns:w="{W}"><w:style w:styleId="List"><w:pPr><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr>'
                  '</w:style></w:styles>')
        body = '<w:p><w:pPr><w:pStyle w:val="List"/><w:numPr><w:numId w:val="0"/></w:numPr></w:pPr><w:r><w:t>Unnumbered</w:t></w:r></w:p>'
        rich = read_docx(docx(body, {'word/styles.xml': styles}))
        self.assertIsNone(rich.sections[0].blocks[-1].numbering)

    def test_native_image_flip_crop_and_effects_are_reported(self):
        import io
        import zipfile
        from convert_fixtures import package
        from hwp_workbench.convert.docx_read import read_docx
        from hwp_workbench.convert.hwpx_read_rich import read_hwpx
        from hwp_workbench.convert.hwpx_write import write_hwpx
        payload = write_hwpx(read_docx(docx()))
        with zipfile.ZipFile(io.BytesIO(payload)) as archive:
            parts = {name: archive.read(name) for name in archive.namelist()}
        parts['Contents/section0.xml'] = parts['Contents/section0.xml'].replace(b'horizontal="0"', b'horizontal="1"').replace(b'<hp:imgClip left="0"', b'<hp:imgClip left="1000"').replace(b'bright="0"', b'bright="40"')
        report = read_hwpx(package(parts)).unsupported
        self.assertEqual(report['imageTransforms'], 1)
        self.assertEqual(report['imageCropping'], 1)
        self.assertEqual(report['imageEffects'], 1)
