"""Authored ZIP and XML adversarial fixtures; no external office applications."""
import io
import struct
import unittest
import zipfile
import zlib
from unittest.mock import patch

from convert_fixtures import package
from fixtures import make_hwpx
from hwp_workbench.convert.package import read_package
from hwp_workbench.document import Document
from hwp_workbench.errors import WorkbenchError


def understated_member(payload, name):
    """Forge both size/CRC fields while retaining all the real compressed data."""
    data = bytearray(payload)
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        info = archive.getinfo(name)
        offset = info.header_offset
    struct.pack_into('<II', data, offset + 14, zlib.crc32(b'A'), info.compress_size)
    struct.pack_into('<I', data, offset + 22, 1)
    offset = data.index(b'PK\x01\x02')
    while True:
        name_size, extra_size, comment_size = struct.unpack_from('<3H', data, offset + 28)
        member = bytes(data[offset + 46:offset + 46 + name_size]).decode('utf-8')
        if member == name:
            struct.pack_into('<I', data, offset + 16, zlib.crc32(b'A'))
            struct.pack_into('<I', data, offset + 24, 1)
            break
        offset += 46 + name_size + extra_size + comment_size
    return bytes(data)


class ZipExpansionTests(unittest.TestCase):
    def test_forged_size_cannot_hide_expansion(self):
        name = 'BinData/oversized.bin'
        for reader, payload in [
            (read_package, package({name: b'A' * (33 * 1024 * 1024)})),
            (Document, make_hwpx({name: b'A' * (33 * 1024 * 1024)})),
        ]:
            with self.subTest(reader=reader.__name__):
                with self.assertRaises(WorkbenchError) as caught:
                    reader(understated_member(payload, name))
                self.assertEqual(caught.exception.code, 'LIMIT')

    def test_actual_total_budget_is_enforced_with_understated_member(self):
        name = 'BinData/forged.bin'
        readers = [
            (read_package, 'hwp_workbench.convert.package', package({name: b'A' * 8192})),
            (Document, 'hwp_workbench.document', make_hwpx({name: b'A' * 8192})),
        ]
        for reader, module, payload in readers:
            with self.subTest(reader=reader.__name__), patch(module + '.MAX_EXPANDED', 4096):
                with self.assertRaises(WorkbenchError) as caught:
                    reader(understated_member(payload, name))
                self.assertEqual(caught.exception.code, 'LIMIT')

    def test_crc_and_deflate_completion_are_checked(self):
        name = 'BinData/data.bin'
        for reader, make in [(read_package, package), (Document, make_hwpx)]:
            payload = make({name: b'A' * 8192})
            with zipfile.ZipFile(io.BytesIO(payload)) as archive:
                offset = archive.getinfo(name).header_offset
            central = payload.index(b'PK\x01\x02')
            while True:
                name_size, extra_size, comment_size = struct.unpack_from('<3H', payload, central + 28)
                member = payload[central + 46:central + 46 + name_size].decode('utf-8')
                if member == name:
                    break
                central += 46 + name_size + extra_size + comment_size
            for corruption in ('crc', 'truncated'):
                with self.subTest(reader=reader.__name__, corruption=corruption):
                    data = bytearray(payload)
                    if corruption == 'crc':
                        struct.pack_into('<I', data, offset + 14, 0)
                        struct.pack_into('<I', data, central + 16, 0)
                    else:
                        size = struct.unpack_from('<I', data, central + 20)[0] - 1
                        struct.pack_into('<I', data, offset + 18, size)
                        struct.pack_into('<I', data, central + 20, size)
                    with self.assertRaises(WorkbenchError) as caught:
                        reader(bytes(data))
                    self.assertEqual(caught.exception.code, 'BAD_ZIP')

    def test_inflated_length_must_match_header_even_below_limit(self):
        name = 'BinData/forged.bin'
        for reader, payload in [
            (read_package, package({name: b'A' * 1000})),
            (Document, make_hwpx({name: b'A' * 1000})),
        ]:
            with self.subTest(reader=reader.__name__):
                with self.assertRaises(WorkbenchError) as caught:
                    reader(understated_member(payload, name))
                self.assertEqual(caught.exception.code, 'BAD_ZIP')


class XmlEncodingTests(unittest.TestCase):
    def test_utf16_entities_are_rejected_in_all_package_parts(self):
        from convert_fixtures import docx
        declaration = '<?xml version="1.0" encoding="UTF-16"?>'
        text = declaration + '<!DOCTYPE a [<!ENTITY x "EXPANDED">]><a>&x;</a>'
        for encoding, bom in [('utf-16le', b'\xff\xfe'), ('utf-16be', b'\xfe\xff')]:
            for prefix in (b'', bom):
                payload = prefix + text.encode(encoding)
                for name in ['word/document.xml', 'word/styles.xml', 'word/unknown.xml', 'word/unknown.rels']:
                    with self.subTest(encoding=encoding, bom=bool(prefix), docx=name):
                        with self.assertRaises(WorkbenchError) as caught:
                            read_package(docx(extra_parts={name: payload}))
                        self.assertEqual(caught.exception.code, 'XML_ENCODING')
                for name in ['Contents/header.xml', 'Contents/section0.xml', 'Contents/content.hpf', 'unknown.xml', 'unknown.rels']:
                    with self.subTest(encoding=encoding, bom=bool(prefix), hwpx=name):
                        with self.assertRaises(WorkbenchError) as caught:
                            Document(make_hwpx({name: payload}))
                        self.assertEqual(caught.exception.code, 'XML_ENCODING')

    def test_encoding_declaration_is_checked_beyond_regex_window(self):
        from hwp_workbench.convert.package import xml
        text = b'<?xml version="1.0"' + b' ' * 250 + b'encoding="ISO-8859-1"?><a>text</a>'
        for parse in (xml, Document.__new__(Document)._xml):
            with self.subTest(parse=parse):
                with self.assertRaises(WorkbenchError) as caught:
                    parse(text)
                self.assertEqual(caught.exception.code, 'XML_ENCODING')

    def test_parser_rejects_dtd_but_allows_declaration_text_in_comments(self):
        from hwp_workbench.convert.package import xml
        for parse in (xml, Document.__new__(Document)._xml):
            with self.subTest(parse=parse):
                with self.assertRaises(WorkbenchError) as caught:
                    parse(b'<!DOCTYPE a [<!ENTITY x "boom">]><a>&x;</a>')
                self.assertEqual(caught.exception.code, 'UNSAFE_XML')
                self.assertEqual(parse(b'<a><!-- <!DOCTYPE a> --></a>').tag, 'a')


class FixtureStabilityTests(unittest.TestCase):
    def test_fixture_package_bytes_do_not_change_with_clock(self):
        with patch('zipfile.time.localtime', return_value=(2026, 1, 1, 0, 0, 0, 3, 1, 0)):
            first = package({'data.bin': b'authored bytes'})
        with patch('zipfile.time.localtime', return_value=(2026, 1, 2, 0, 0, 0, 4, 2, 0)):
            second = package({'data.bin': b'authored bytes'})
        self.assertEqual(first, second)
