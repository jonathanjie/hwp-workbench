"""HWP record structure and native edit regression tests."""
import struct
import unittest

from fixtures import cfb_from_streams, make_hwp, record
from hwp_workbench.cfb import CompoundFile
from hwp_workbench.document import Document


class ParagraphStructureTests(unittest.TestCase):
    def test_nodes_retain_record_owner_and_code_unit_bounds(self):
        document = Document(make_hwp(False))
        node = document.nodes[0]
        self.assertEqual(node.id, 'BodyText/Section0#record:1:text:0')
        self.assertEqual(node.paragraphRecordIndex, 0)
        self.assertEqual(node.textStart, 16)
        self.assertEqual(node.textEnd, 16 + len(node.text.encode('utf-16le')) // 2)
        section = document.hwp_sections[node.part]
        self.assertEqual(section.paragraphs[0].children, [1, 2, 3, 4, 6])
        self.assertEqual(b''.join(item.original for item in section.records), document.sections[node.part])

    def test_nested_paragraph_has_its_own_owner(self):
        streams = dict(CompoundFile(make_hwp(False)).streams)
        streams['BodyText/Section0'] = (
            record(66, b'\0' * 22)
            + record(67, 'A😀\r'.encode('utf-16le'), 1)
            + record(71, b' lbt' + b'\0' * 20, 1)
            + record(66, b'\0' * 22, 2)
            + record(67, 'cell\r'.encode('utf-16le'), 3)
        )
        nodes = Document(cfb_from_streams(streams)).nodes
        self.assertEqual([node.paragraphRecordIndex for node in nodes], [0, 3])
        self.assertEqual([(node.textStart, node.textEnd) for node in nodes], [(0, 3), (0, 4)])


def paragraph_fixture(text='abcdef', shapes=((0, 10), (2, 20), (4, 30), (6, 40))):
    streams = dict(CompoundFile(make_hwp(False)).streams)
    payload = (text + '\r').encode('utf-16le')
    header = struct.pack('<IIHBBHHHI', len(payload) // 2 | 0x80000000,
                         0x12345678, 7, 8, 9, len(shapes), 1, 3, 0xabcdef01) + b'EXTRA'
    text_units = len(payload) // 2 - 1
    line_data = b''.join(struct.pack('<I', start) + bytes([start + 1]) * 32
                         for start in (0, min(3, text_units), min(6, text_units)))
    streams['BodyText/Section0'] = (
        record(66, header) + record(67, payload, 1)
        + record(68, b''.join(struct.pack('<II', *shape) for shape in shapes), 1)
        + record(69, line_data, 1)
        + record(70, struct.pack('<III', min(2, text_units), min(6, text_units + 1), 0xfeedbeef), 1)
        + record(99, b'opaque child', 1)
    )
    return cfb_from_streams(streams)


def set_text(document, node, text):
    return document.patch({
        'schemaVersion': 1, 'baseSha256': document.hash,
        'operations': [{'op': 'set_text', 'nodeId': node.id,
                        'expectedText': node.text, 'text': text}],
    })


class VariableLengthTests(unittest.TestCase):
    def test_grow_updates_metadata_and_preserves_other_fields(self):
        from hwp_workbench.hwp_records import parse_section
        document = Document(paragraph_fixture())
        self.assertTrue(document.inspect()['capabilities']['lengthChangingText'])
        set_text(document, document.nodes[0], 'ABCDEFGHIJ')
        saved = Document(document.serialize(True))
        self.assertEqual(saved.nodes[0].text, 'ABCDEFGHIJ')
        old = document.hwp_sections['BodyText/Section0'].records
        new = parse_section(saved.sections['BodyText/Section0']).records
        self.assertEqual(struct.unpack_from('<I', new[0].data)[0], 0x8000000b)
        self.assertEqual(new[0].data[4:], old[0].data[4:])
        self.assertEqual(struct.unpack('<8I', new[2].data), (0, 10, 2, 20, 4, 30, 10, 40))
        self.assertEqual(struct.unpack('<III', new[4].data), (2, 10, 0xfeedbeef))
        self.assertEqual([struct.unpack_from('<I', new[3].data, offset)[0]
                          for offset in (0, 36, 72)], [0, 3, 10])
        for offset in (0, 36, 72):
            self.assertEqual(new[3].data[offset + 4:offset + 36], old[3].data[offset + 4:offset + 36])
        self.assertEqual(new[5].original, old[5].original)
        self.assertIn('HWP_REFLOW_UNVERIFIED', [warning['code'] for warning in saved.warnings])
        self.assertNotIn('HWP_EQUAL_LENGTH_ONLY', [warning['code'] for warning in saved.warnings])

    def test_shrink_keeps_shape_of_surviving_terminator(self):
        from hwp_workbench.hwp_records import parse_section
        document = Document(paragraph_fixture())
        set_text(document, document.nodes[0], 'XY')
        saved = Document(document.serialize(True))
        records = parse_section(saved.sections['BodyText/Section0']).records
        self.assertEqual(struct.unpack('<4I', records[2].data), (0, 10, 2, 40))
        self.assertEqual(struct.unpack_from('<H', records[0].data, 12)[0], 2)
        self.assertEqual(struct.unpack('<III', records[4].data), (2, 2, 0xfeedbeef))
        self.assertEqual(records[0].data[4:12], document.hwp_sections['BodyText/Section0'].records[0].data[4:12])
        self.assertEqual(records[0].data[14:], document.hwp_sections['BodyText/Section0'].records[0].data[14:])

    def test_generated_fixture_accepts_grow_and_shrink(self):
        for replacement in ('longer text 😀 with more words', '짧음'):
            with self.subTest(text=replacement):
                document = Document(make_hwp())
                set_text(document, document.nodes[0], replacement)
                self.assertEqual(Document(document.serialize(True)).nodes[0].text, replacement)


class EdgeCaseTests(unittest.TestCase):
    def test_deletion_keeps_surviving_control_and_suffix_shapes(self):
        cases = [
            ('ab\ncd', ((0, 10), (2, 20)), 0, ((0, 20),)),
            ('xy\nab\ncd', ((0, 10), (3, 20), (5, 30)), 1,
             ((0, 10), (3, 30))),
            ('xy\nab', ((0, 10), (3, 20), (5, 30)), 1,
             ((0, 10), (3, 30))),
            ('ab\ncd', ((0, 10), (2, 20), (2, 30)), 0,
             ((0, 20), (0, 30))),
            ('ab\ncd', ((0, 10), (0, 11), (2, 20)), 0,
             ((0, 10), (0, 11), (0, 20))),
        ]
        for text, shapes, deleted_run, expected in cases:
            with self.subTest(text=text, shapes=shapes):
                document = Document(paragraph_fixture(text, shapes))
                set_text(document, document.nodes[deleted_run], '')
                saved = Document(document.serialize(True))
                records = saved.hwp_sections['BodyText/Section0'].records
                actual = tuple(struct.iter_unpack('<II', records[2].data))
                self.assertEqual(actual, expected)
                self.assertEqual(records[1].data,
                                 (text[:document.nodes[deleted_run].textStart]
                                  + text[document.nodes[deleted_run].textEnd:]
                                  + '\r').encode('utf-16le'))

    def test_empty_text_node_accepts_insertion(self):
        document = Document(paragraph_fixture('', shapes=((0, 10),)))
        self.assertEqual(len(document.nodes), 1)
        self.assertEqual(document.nodes[0].text, '')
        self.assertEqual((document.nodes[0].textStart, document.nodes[0].textEnd), (0, 0))
        set_text(document, document.nodes[0], 'new 😀 text')
        saved = Document(document.serialize(True))
        self.assertEqual(saved.nodes[0].text, 'new 😀 text')

    def test_text_to_empty_retains_terminator_and_all_streams(self):
        document = Document(make_hwp())
        set_text(document, document.nodes[0], '')
        saved = Document(document.serialize(True))
        self.assertEqual(saved.nodes[0].text, '')
        self.assertTrue(saved.hwp_sections['BodyText/Section0'].records[1].data.endswith(b'\r\0'))
        for part in document.parts:
            if part != 'BodyText/Section0':
                self.assertEqual(saved.parts[part], document.parts[part])

    def test_two_edits_compose_across_runs_and_shape_boundaries(self):
        document = Document(paragraph_fixture('ab\ncd', shapes=((0, 10), (1, 20), (2, 30), (3, 40), (5, 50))))
        first, second = document.nodes
        operations = [{'op': 'set_text', 'nodeId': node.id, 'expectedText': node.text, 'text': text}
                      for node, text in ((first, 'ABCDE'), (second, 'Z'))]
        document.patch({'schemaVersion': 1, 'baseSha256': document.hash, 'operations': operations})
        saved = Document(document.serialize(True))
        self.assertEqual([node.text for node in saved.nodes], ['ABCDE', 'Z'])
        records = saved.hwp_sections['BodyText/Section0'].records
        self.assertEqual(records[1].data, 'ABCDE\nZ\r'.encode('utf-16le'))
        self.assertEqual(struct.unpack('<10I', records[2].data), (0, 10, 1, 20, 5, 30, 6, 40, 7, 50))
        self.assertEqual(struct.unpack('<III', records[4].data), (5, 8, 0xfeedbeef))

    def test_deleting_one_of_two_runs_does_not_fail_old_node_id_verification(self):
        document = Document(paragraph_fixture('ab\ncd', shapes=((0, 10), (3, 20))))
        first, second = document.nodes
        set_text(document, first, '')
        set_text(document, second, 'longer')
        saved = Document(document.serialize(True))
        self.assertEqual(saved.hwp_sections['BodyText/Section0'].records[1].data, '\nlonger\r'.encode('utf-16le'))
        self.assertEqual([node.text for node in saved.nodes], ['longer'])

    def test_record_grows_and_shrinks_across_extended_size_boundary(self):
        document = Document(make_hwp(False))
        set_text(document, document.nodes[0], 'X' * 2032)
        saved = Document(document.serialize(True))
        item = saved.hwp_sections['BodyText/Section0'].records[1]
        self.assertGreaterEqual(len(item.data), 4095)
        self.assertEqual(struct.unpack_from('<I', item.original)[0] >> 20, 4095)
        self.assertEqual(struct.unpack_from('<I', item.original, 4)[0], len(item.data))
        set_text(saved, saved.nodes[0], 'short')
        shrunk = Document(saved.serialize(True))
        item = shrunk.hwp_sections['BodyText/Section0'].records[1]
        self.assertEqual(struct.unpack_from('<I', item.original)[0] >> 20, len(item.data))
        self.assertEqual(shrunk.nodes[0].text, 'short')

    def test_control_code_insertions_are_refused_with_hint(self):
        from hwp_workbench.errors import WorkbenchError
        document = Document(make_hwp())
        for text in ('before\tother', 'before\nother', '\r', '\x02'):
            with self.subTest(text=text), self.assertRaises(WorkbenchError) as raised:
                set_text(document, document.nodes[0], text)
            self.assertEqual(raised.exception.code, 'UNSUPPORTED_ELEMENT')
            self.assertIn('control', str(raised.exception))
        self.assertEqual(document.edits, {})

    def test_edit_must_not_touch_adjacent_or_extended_control_units(self):
        from hwp_workbench.errors import WorkbenchError
        for start_change, end_change in ((0, 2), (-2, 0), (-14, 0)):
            with self.subTest(start=start_change, end=end_change):
                document = Document(make_hwp(False))
                node = document.nodes[0]
                position = document.positions[node.id]
                position['start'] += start_change
                position['end'] += end_change
                position['textStart'] += start_change // 2
                position['textEnd'] += end_change // 2
                with self.assertRaises(WorkbenchError) as raised:
                    set_text(document, node, 'changed')
                self.assertEqual(raised.exception.code, 'UNSUPPORTED_ELEMENT')
                self.assertIn('control', str(raised.exception))
                self.assertEqual(document.edits, {})

    def test_noop_patch_and_revert_are_byte_identical(self):
        for compressed in (False, True):
            payload = make_hwp(compressed)
            document = Document(payload)
            node = document.nodes[0]
            set_text(document, node, node.text)
            self.assertEqual(document.serialize(), payload)
            set_text(document, node, 'changed length')
            document.patch({'schemaVersion': 1, 'baseSha256': document.hash, 'operations': [
                {'op': 'set_text', 'nodeId': node.id, 'expectedText': 'changed length', 'text': node.text}]})
            self.assertEqual(document.serialize(), payload)

    def test_length_changes_refresh_optional_compression_trailer(self):
        import zlib
        for trailer in (False, True):
            for replacement in ('longer 😀 plain text replacement', 'x', ''):
                with self.subTest(trailer=trailer, text=replacement):
                    streams = dict(CompoundFile(make_hwp()).streams)
                    if trailer:
                        raw = zlib.decompress(streams['BodyText/Section0'], -15)
                        streams['BodyText/Section0'] += struct.pack('<II', zlib.crc32(raw), len(raw))
                    document = Document(cfb_from_streams(streams))
                    set_text(document, document.nodes[0], replacement)
                    saved = Document(document.serialize(True))
                    self.assertEqual(saved.nodes[0].text, replacement)
                    decoder = zlib.decompressobj(-15)
                    raw = decoder.decompress(saved.parts['BodyText/Section0'])
                    expected = struct.pack('<II', zlib.crc32(raw), len(raw)) if trailer else b''
                    self.assertEqual(decoder.unused_data, expected)
                    for part, original in streams.items():
                        if part != 'BodyText/Section0':
                            self.assertEqual(saved.parts[part], original)

    def test_shared_capabilities_describe_variable_length_hwp(self):
        from hwp_workbench.service import CAPABILITIES
        self.assertIn('variable length', CAPABILITIES['formats']['hwp'])
        self.assertIn('Hancom reflow unverified', CAPABILITIES['formats']['hwp'])


class IndependentLengthTests(unittest.TestCase):
    def setUp(self):
        from hwp_workbench import independent
        self.bridge = independent
        observed = independent.probe(make_hwp())
        if observed.get('available') is False:
            self.skipTest(observed['reason'])

    def assert_paragraph(self, payload, text):
        observed = self.bridge.probe(payload)
        self.assertEqual(observed['sections'][0]['paragraphs'][0]['text'], text)
        self.assertGreaterEqual(observed['pageCount'], 1)

    def test_generated_hwp_variants_reopen_with_exact_paragraph_text(self):
        import zlib
        from pathlib import Path
        generated = Path(__file__).resolve().parents[1] / 'fixtures/generated/legacy-review.hwp'
        for compressed, trailer in ((False, False), (True, False), (True, True)):
            streams = dict(CompoundFile(generated.read_bytes() if compressed else make_hwp(False)).streams)
            if trailer:
                raw = zlib.decompress(streams['BodyText/Section0'], -15)
                streams['BodyText/Section0'] += struct.pack('<II', zlib.crc32(raw), len(raw))
            payload = cfb_from_streams(streams)
            for replacement in ('longer replacement 😀 with many words', 'x', '', 'X' * 2032):
                with self.subTest(compressed=compressed, trailer=trailer, textLength=len(replacement)):
                    document = Document(payload)
                    set_text(document, document.nodes[0], replacement)
                    saved = document.serialize(True)
                    self.assert_paragraph(saved, replacement)
                    # Reuse the saved empty paragraph for insertion; verify that
                    # extended records can also return to short record headers.
                    reopened = Document(saved)
                    if not replacement or len(replacement) > 2000:
                        set_text(reopened, reopened.nodes[0], 'inserted 😀')
                        self.assert_paragraph(reopened.serialize(True), 'inserted 😀')

    def test_shape_boundary_and_two_run_edits_reopen_exactly(self):
        for replacement in ('ABCDEFGHIJ', 'XY', ''):
            with self.subTest(text=replacement):
                document = Document(paragraph_fixture())
                set_text(document, document.nodes[0], replacement)
                self.assert_paragraph(document.serialize(True), replacement)
        for replacements in (('ABCDE', 'Z'), ('', 'longer'), ('longer', '')):
            with self.subTest(replacements=replacements):
                document = Document(paragraph_fixture('ab\ncd', shapes=((0, 10), (3, 20))))
                for node, replacement in zip(document.nodes, replacements):
                    set_text(document, node, replacement)
                self.assert_paragraph(document.serialize(True), '\n'.join(replacements))


class FailClosedRecordTests(unittest.TestCase):
    def test_bad_paragraph_metadata_rejects_atomic_patch(self):
        from hwp_workbench.errors import WorkbenchError
        mutations = [(0, '<I', 1), (12, '<H', 99), (14, '<H', 99), (16, '<H', 99)]
        for offset, format, value in mutations:
            with self.subTest(offset=offset):
                source = Document(make_hwp(False))
                raw = bytearray(source.sections['BodyText/Section0'])
                struct.pack_into(format, raw, source.hwp_sections['BodyText/Section0'].records[0].payload_start + offset, value)
                streams = dict(source.parts)
                streams['BodyText/Section0'] = bytes(raw)
                document = Document(cfb_from_streams(streams))
                with self.assertRaises(WorkbenchError) as raised:
                    set_text(document, document.nodes[0], 'changed')
                self.assertEqual(raised.exception.code, 'UNSUPPORTED_ELEMENT')
                self.assertEqual(document.edits, {})
                self.assertEqual(document.serialize(), document.original)

    def test_untouched_noncanonical_extended_header_is_preserved(self):
        streams = dict(CompoundFile(make_hwp(False)).streams)
        opaque = struct.pack('<II', 99 | (1 << 10) | (4095 << 20), 3) + b'raw'
        streams['BodyText/Section0'] += opaque
        document = Document(cfb_from_streams(streams))
        set_text(document, document.nodes[0], 'longer text 😀')
        saved = Document(document.serialize(True))
        self.assertEqual(saved.hwp_sections['BodyText/Section0'].records[-1].original, opaque)

    def test_edit_limit_is_checked_before_large_record_allocation(self):
        from unittest.mock import patch
        from hwp_workbench.errors import WorkbenchError
        import hwp_workbench.hwp_records as native
        document = Document(make_hwp(False))
        set_text(document, document.nodes[0], 'X' * 1000)
        with patch.object(native, 'MAX_PART', 100):
            with self.assertRaises(WorkbenchError) as raised:
                document.serialize(True)
        self.assertEqual(raised.exception.code, 'LIMIT')

    def test_existing_duplicate_shape_starts_survive_when_not_collapsed(self):
        document = Document(paragraph_fixture(shapes=((0, 10), (0, 20), (4, 30), (6, 40))))
        set_text(document, document.nodes[0], 'ABCDEFGHIJ')
        saved = Document(document.serialize(True))
        records = saved.hwp_sections['BodyText/Section0'].records
        self.assertEqual(struct.unpack('<8I', records[2].data), (0, 10, 0, 20, 4, 30, 10, 40))
