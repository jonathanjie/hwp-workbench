"""Native node addresses must use rhwp's Unicode paragraph text axis."""
import json
import os
from pathlib import Path
import shutil
import struct
import unittest

from fixtures import cfb_from_streams, make_hwp, make_hwpx, record
from hwp_workbench.cfb import CompoundFile
from hwp_workbench.document import Document, HP
from hwp_workbench import independent

ROOT = Path(__file__).resolve().parents[1]


class AnchorTests(unittest.TestCase):
    def assert_body_ranges(self, payload):
        observed = independent.probe(payload)
        if observed.get('available') is False:
            self.skipTest(observed['reason'])
        document = Document(payload)
        body = [node for node in document.nodes if node.anchor['charOffset'] is not None]
        request = json.dumps({'nodes': [{'id': node.id, 'anchor': node.anchor, 'count': len(node.text)}
                                        for node in body]}).encode()
        node = shutil.which(os.environ.get('HWP_WORKBENCH_NODE') or 'node')
        result = independent._run_bridge([node, str(ROOT / 'tests/rhwp-anchor-ranges.mjs')],
                                        struct.pack('<I', len(request)) + request + payload)
        self.assertEqual(result['ranges'], {node.id: node.text for node in body})

    def test_hwpx_runs_and_cell_share_top_level_owner(self):
        xml = f'''<sec xmlns:hp="{HP}"><hp:p><hp:run><hp:t>A😀</hp:t><hp:t>B</hp:t></hp:run></hp:p>
        <hp:p><hp:run><hp:tbl><hp:tr><hp:tc><hp:p><hp:run><hp:t>cell</hp:t></hp:run></hp:p></hp:tc></hp:tr></hp:tbl><hp:t>after</hp:t></hp:run></hp:p>
        <hp:p><hp:run><hp:t>last</hp:t></hp:run></hp:p></sec>'''
        nodes = Document(make_hwpx(section=xml)).nodes
        self.assertEqual([node.anchor for node in nodes], [
            {'section': 0, 'paragraph': 0, 'charOffset': 0, 'inCell': False},
            {'section': 0, 'paragraph': 0, 'charOffset': 2, 'inCell': False},
            {'section': 0, 'paragraph': 1, 'charOffset': None, 'inCell': True},
            {'section': 0, 'paragraph': 1, 'charOffset': 0, 'inCell': False},
            {'section': 0, 'paragraph': 2, 'charOffset': 0, 'inCell': False},
        ])

    def test_hwp_control_offsets_count_scalars(self):
        streams = dict(CompoundFile(make_hwp(False)).streams)
        control = lambda code: struct.pack('<H', code) + b'\0' * 14
        text = 'A😀'.encode('utf-16le') + control(9) + b'B\0' + control(2) + control(18) + b'C\0\r\0'
        streams['BodyText/Section0'] = record(66, b'\0' * 22) + record(67, text, 1)
        payload = cfb_from_streams(streams)
        nodes = Document(payload).nodes
        self.assertEqual([node.anchor['charOffset'] for node in nodes], [0, 3, 5])
        self.assert_body_ranges(payload)

    def test_hwp_nested_cell_does_not_increment_body_index(self):
        streams = dict(CompoundFile(make_hwp(False)).streams)
        streams['BodyText/Section0'] = (
            record(66, b'\0' * 22) + record(67, b'A\0\r\0', 1)
            + record(71, b' lbt' + b'\0' * 20, 1) + record(77, b'\0' * 20, 2) + record(72, b'\0' * 20, 2)
            + record(66, b'\0' * 22, 2) + record(67, 'cell\r'.encode('utf-16le'), 3)
            + record(66, b'\0' * 22) + record(67, b'B\0\r\0', 1))
        nodes = Document(cfb_from_streams(streams)).nodes
        self.assertEqual([node.anchor['paragraph'] for node in nodes], [0, 0, 1])
        self.assertEqual(nodes[1].anchor['charOffset'], None)
        self.assertTrue(nodes[1].anchor['inCell'])
        self.assertEqual(nodes[1].role, 'table-cell')

    def test_every_fixture_body_anchor_matches_rhwp_text_range(self):
        paths = list((ROOT / 'fixtures/generated').glob('*.hwp*'))
        manifest = json.loads((ROOT / 'fixtures/public-manifest.json').read_text())
        paths.extend(ROOT / 'fixtures/public' / item['name'] for item in manifest['fixtures'])
        availability = independent.probe(make_hwpx())
        if availability.get('available') is False:
            self.skipTest(availability['reason'])
        for path in paths:
            if not path.is_file():
                with self.subTest(file=path.name):
                    self.skipTest('Public fixture not downloaded: ' + path.name)
                continue
            with self.subTest(file=path.name):
                payload = path.read_bytes()
                document = Document(payload)
                self.assertTrue(all(node.anchor is not None for node in document.nodes))
                self.assert_body_ranges(payload)

    def test_hwpx_inline_controls_and_runs_match_direct_ranges(self):
        xml = f'''<sec xmlns:hp="{HP}"><hp:p><hp:run><hp:t>A😀<hp:tab/>B<hp:nbSpace/>C<hp:hyphen/>D</hp:t><hp:t>tail</hp:t></hp:run></hp:p></sec>'''
        payload = make_hwpx(section=xml)
        document = Document(payload)
        observed = independent.probe(payload)
        if observed.get('available') is False:
            self.skipTest(observed['reason'])
        self.assertEqual(document.nodes[1].anchor['charOffset'], 8)
        self.assertEqual(observed['sections'][0]['paragraphs'][0]['text'], ''.join(node.text for node in document.nodes))

    def test_paired_hwpx_controls_match_rhwp_offsets(self):
        xml = f'''<sec xmlns:hp="{HP}"><hp:p><hp:run><hp:t>A</hp:t><hp:tab></hp:tab><hp:t>B<hp:tab></hp:tab>C</hp:t></hp:run></hp:p></sec>'''
        payload = make_hwpx(section=xml)
        document = Document(payload)
        self.assertEqual(document.nodes[1].anchor['charOffset'], 1)
        self.assertEqual(document.nodes[1].text, 'BC')
        self.assert_body_ranges(payload)

    def test_hwp_table_caption_is_not_cell_text(self):
        streams = dict(CompoundFile(make_hwp(False)).streams)
        streams['BodyText/Section0'] = (
            record(66, b'\0' * 22)
            + record(71, b' lbt' + b'\0' * 20, 1)
            + record(72, b'\0' * 20, 2)
            + record(66, b'\0' * 22, 2) + record(67, 'caption\r'.encode('utf-16le'), 3)
            + record(77, b'\0' * 20, 2)
            + record(72, b'\0' * 20, 2)
            + record(66, b'\0' * 22, 2) + record(67, 'cell\r'.encode('utf-16le'), 3))
        nodes = Document(cfb_from_streams(streams)).nodes
        self.assertFalse(nodes[0].anchor['inCell'])
        self.assertTrue(nodes[1].anchor['inCell'])
        self.assertIsNone(nodes[0].anchor['charOffset'])

    def test_paired_auto_number_counts_inside_ctrl(self):
        xml = f'''<sec xmlns:hp="{HP}"><hp:p><hp:run><hp:ctrl><hp:autoNum num="1" numType="FOOTNOTE"><hp:autoNumFormat type="DIGIT"/></hp:autoNum></hp:ctrl><hp:t>B</hp:t></hp:run></hp:p></sec>'''
        payload = make_hwpx(section=xml)
        self.assertEqual(Document(payload).nodes[0].anchor['charOffset'], 1)
        self.assert_body_ranges(payload)
