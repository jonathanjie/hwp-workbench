"""Shared bounded ZIP/XML I/O; no package member is extracted to disk."""
import io
import posixpath
import re
import stat
import zipfile
import zlib
from pathlib import PurePosixPath
from xml.parsers import expat
from xml.etree import ElementTree as ET

from ..errors import MAX_FILE, MAX_PART, MAX_EXPANDED, MAX_NODES, WorkbenchError, require

MAX_DEPTH = 128
MAX_COLUMNS = 1000
MAX_GRID_WORK = 1_000_000


def xml(payload):
    require(len(payload) <= MAX_PART, 'LIMIT', 'XML part exceeds 32 MiB.')
    try:
        text = payload.decode('utf-8-sig')
    except UnicodeError as error:
        raise WorkbenchError('XML_ENCODING', 'Conversion XML must be UTF-8.') from error
    require(not re.search(r'<!\s*(DOCTYPE|ENTITY)\b', text, re.I), 'UNSAFE_XML', 'DTD and entities are prohibited.')
    require(not re.search(r'encoding\s*=\s*[\"\'](?!utf-8)[^\"\']+', text[:200], re.I),
            'XML_ENCODING', 'Conversion XML must declare UTF-8.')
    depth = 0
    nodes = 0
    parser = expat.ParserCreate()

    def start(name, attrs):
        nonlocal depth, nodes
        depth += 1
        nodes += 1
        require(depth <= MAX_DEPTH and nodes <= MAX_NODES * 10, 'LIMIT', 'XML complexity exceeds limits.')

    def end(name):
        nonlocal depth
        depth -= 1

    parser.StartElementHandler = start
    parser.EndElementHandler = end
    try:
        parser.Parse(payload, True)
        return ET.fromstring(payload)
    except (expat.ExpatError, ET.ParseError) as error:
        raise WorkbenchError('BAD_XML', str(error)) from error


def read_package(payload):
    require(isinstance(payload, bytes) and 0 < len(payload) <= MAX_FILE, 'LIMIT', 'File must be 1 byte to 64 MiB.')
    parts = {}
    expanded = 0
    try:
        with zipfile.ZipFile(io.BytesIO(payload)) as archive:
            require(len(archive.infolist()) <= 2048, 'LIMIT', 'Too many ZIP members.')
            for info in archive.infolist():
                name = info.filename
                path = PurePosixPath(name)
                require(bool(name) and not path.is_absolute() and '..' not in path.parts
                        and '\\' not in name and '\x00' not in name and name not in parts,
                        'UNSAFE_ZIP', 'Unsafe or duplicate ZIP member.')
                require(not stat.S_ISLNK(info.external_attr >> 16), 'UNSAFE_ZIP', 'ZIP symlinks are prohibited.')
                require(not info.flag_bits & 1, 'ENCRYPTED', 'Encrypted ZIP members are not supported.')
                require(info.compress_type in {zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED}, 'UNSAFE_ZIP', 'Unsupported ZIP compression.')
                expanded += info.file_size
                require(info.file_size <= MAX_PART and expanded <= MAX_EXPANDED, 'LIMIT', 'Expanded ZIP exceeds limits.')
                parts[name] = archive.read(info)
    except (zipfile.BadZipFile, RuntimeError, zlib.error, NotImplementedError) as error:
        raise WorkbenchError('BAD_ZIP', str(error)) from error
    # Validate all XML, including unsupported parts, before any optional native tool.
    trees = {name: xml(data) for name, data in parts.items() if name.endswith(('.xml', '.rels', '.hpf'))}
    return parts, trees


def write_package(parts):
    require(len(parts) <= 2048, 'LIMIT', 'Too many output ZIP members.')
    require(all(len(data) <= MAX_PART for data in parts.values())
            and sum(map(len, parts.values())) <= MAX_EXPANDED, 'LIMIT', 'Expanded output exceeds limits.')
    output = io.BytesIO()
    with zipfile.ZipFile(output, 'w') as archive:
        for name, data in parts.items():
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED if name == 'mimetype' else zipfile.ZIP_DEFLATED
            archive.writestr(info, data)
    result = output.getvalue()
    require(len(result) <= MAX_FILE, 'LIMIT', 'Output exceeds 64 MiB.')
    return result


def element(parent, namespace, tag, **attrs):
    return ET.SubElement(parent, '{' + namespace + '}' + tag, {key: str(value) for key, value in attrs.items()})


def encoded(root):
    return ET.tostring(root, encoding='utf-8', xml_declaration=True)


def integer(value, default=0, *, minimum=-10_000_000, maximum=10_000_000):
    if value is None:
        return default
    try:
        result = int(value)
    except (ValueError, TypeError) as error:
        raise WorkbenchError('BAD_XML', 'Invalid numeric document attribute.') from error
    require(minimum <= result <= maximum, 'LIMIT', 'Document numeric attribute exceeds limits.')
    return result


def member_target(part, target):
    # Relative relationships may use ../ within the package; never leave its root.
    require(isinstance(target, str) and bool(target) and '\\' not in target
            and '\x00' not in target and ':' not in target, 'UNSAFE_ZIP', 'Unsafe internal relationship.')
    resolved = posixpath.normpath(posixpath.join(posixpath.dirname(part), target))
    require(not resolved.startswith(('/', '../')) and resolved != '..', 'UNSAFE_ZIP', 'Relationship leaves package.')
    return resolved
