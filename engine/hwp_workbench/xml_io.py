"""UTF-8-only XML parsing with parser-level declaration and complexity checks."""
from xml.etree import ElementTree as ET
from xml.parsers import expat

from .errors import MAX_NODES, MAX_PART, WorkbenchError, require

MAX_DEPTH = 128


def utf8_xml_text(payload):
    require(len(payload) <= MAX_PART, 'LIMIT', 'XML part exceeds 32 MiB.')
    require(not payload.startswith((b'\xff\xfe', b'\xfe\xff')) and b'\0' not in payload,
            'XML_ENCODING', 'XML must be UTF-8 without NUL bytes.')
    try:
        return payload.decode('utf-8-sig')
    except UnicodeError as error:
        raise WorkbenchError('XML_ENCODING', 'XML must be UTF-8.') from error


def restrict_xml_parser(parser):
    """Install restrictions on every parser, including the native token parser."""
    def declaration(version, encoding, standalone):
        require(encoding is None or encoding.lower() == 'utf-8',
                'XML_ENCODING', 'XML encoding must declare UTF-8.')

    def reject(*args):
        raise WorkbenchError('UNSAFE_XML', 'DTD and entity declarations are prohibited.')

    parser.XmlDeclHandler = declaration
    parser.StartDoctypeDeclHandler = reject
    parser.EntityDeclHandler = reject
    parser.ExternalEntityRefHandler = reject
    parser.UnparsedEntityDeclHandler = reject


def xml_tree(payload):
    text = utf8_xml_text(payload)
    parser = expat.ParserCreate()
    restrict_xml_parser(parser)
    depth = 0
    nodes = 0

    def start(name, attrs):
        nonlocal depth, nodes
        depth += 1
        nodes += 1
        require(depth <= MAX_DEPTH and nodes <= MAX_NODES * 10,
                'LIMIT', 'XML complexity exceeds limits.')

    def end(name):
        nonlocal depth
        depth -= 1

    parser.StartElementHandler = start
    parser.EndElementHandler = end
    try:
        # Parse only the validated UTF-8 representation. The declaration
        # handler runs before any element or entity content is consumed.
        parser.Parse(text, True)
        return ET.fromstring(text)
    except (expat.ExpatError, ET.ParseError) as error:
        raise WorkbenchError('BAD_XML', str(error)) from error
