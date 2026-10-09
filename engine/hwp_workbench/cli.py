"""Executable without third-party Python dependencies."""
import argparse
import json
import sys

from .errors import WorkbenchError, require, error_details
from .service import invoke, CAPABILITIES
from .mcp import serve
from .workspace import Workspace
from . import skill_install, mcp_config, desktop_open


class ArgumentParser(argparse.ArgumentParser):
    def error(self, message):
        raise WorkbenchError('BAD_ARGUMENTS', message)


COMMAND_HELP = {
    'convert': 'Report an explicit, lossy format export; optionally save a new file.',
    'session': 'Read Agent link state and proposal outcomes without writing.',
    'propose': 'Write a validated patch for desktop review without changing the document.',
    'inspect': 'Inspect native text nodes and compatibility warnings.',
    'extract': 'Extract document text without node references.',
    'validate': 'Check bounded native container and parse integrity.',
    'plan': 'Validate a patch and show changes without writing.',
    'apply': 'Apply a reviewed patch to a new native document file.',
    'read': 'Read paged text with short node references.',
    'search': 'Find text and return native node IDs and offsets.',
    'replace': 'Preview term replacements or save a new document copy.',
    'fill': 'Preview form values or save a new document copy.',
    'set': 'Preview one text-node edit or save a new document copy.',
}


def _parser() -> argparse.ArgumentParser:
    parser = ArgumentParser(prog='hwp-workbench', description='Local HWP/HWPX preservation editor — development alpha.')
    parser.add_argument('--version', action='version', version=CAPABILITIES['version'], help='Show the application version.')
    sub = parser.add_subparsers(dest='command', required=True, help='Operation to run.')
    sub.add_parser('capabilities', help='List supported operations and safety limits.')
    for command in CAPABILITIES['operations']:
        operation = sub.add_parser(command, help=COMMAND_HELP[command], description=COMMAND_HELP[command])
        operation.add_argument('path', help='Source HWP/HWPX path inside the workspace.')
        operation.add_argument('--root', default='.', help='Allowed workspace (default: current directory).')
        if command in {'plan', 'apply', 'propose'}:
            operation.add_argument('--patch', required=True, help='JSON patch or edit-preview file inside workspace.')
        if command in {'apply', 'replace', 'fill', 'set'}:
            operation.add_argument('-o', '--output', required=command == 'apply', help='New same-format output path; must not exist.')
            operation.add_argument('--acknowledge-risks', action='store_true', help='Acknowledge compatibility warnings before saving.')
        if command == 'convert':
            operation.add_argument('--to', choices=['hwp', 'hwpx', 'docx'], required=True, help='Requested export format.')
            operation.add_argument('-o', '--output', help='New output path with the target extension; must not exist.')
            operation.add_argument('--acknowledge-risks', action='store_true', help='Acknowledge conversion losses before writing.')
        if command == 'propose':
            operation.add_argument('--title', required=True, help='Title shown to the person reviewing the proposal.')
            operation.add_argument('--author', default='agent', help='Proposal author (default: agent).')
            operation.add_argument('--notes', help='Optional review notes.')
        if command == 'inspect':
            operation.add_argument('--compact', action='store_true', help='Omit the structure view and package parts.')
        if command in {'extract', 'read'}:
            operation.add_argument('--format', choices=['text', 'json'], default='json', help='Output format (default: json).')
        if command == 'read':
            operation.add_argument('--offset', type=int, default=0, help='First text-node index (default: 0).')
            operation.add_argument('--limit', type=int, default=200, help='Maximum text nodes (default: 200; maximum: 1000).')
        if command in {'search', 'replace'}:
            operation.add_argument('--regex', action='store_true', help='Use bounded fixed-width regex syntax.')
            case = operation.add_mutually_exclusive_group()
            case.add_argument('--case-sensitive', dest='case_sensitive', action='store_true', help='Match case exactly (replace default).')
            case.add_argument('--ignore-case', dest='case_sensitive', action='store_false', help='Ignore case (search default).')
            operation.set_defaults(case_sensitive=command == 'replace')
            operation.add_argument('--limit', type=int, default=100 if command == 'search' else 1000, help='Maximum matches (maximum: 1000).')
        if command == 'search':
            operation.add_argument('query_text', nargs='?', help='Text to find; alternative to --query.')
            operation.add_argument('--query', help='Text to find; alternative to positional query.')
        if command == 'replace':
            operation.add_argument('--find', required=True, help='Term or supported regex to find.')
            operation.add_argument('--replace-with', required=True, help='Literal replacement text.')
            operation.add_argument('--node-id', action='append', help='Full ID or short reference; repeat to restrict targets.')
        if command == 'fill':
            fields = operation.add_mutually_exclusive_group(required=True)
            fields.add_argument('--json', help='JSON label/value object file inside workspace.')
            fields.add_argument('--field', action='append', help='LABEL=VALUE; repeat for each field.')
        if command == 'set':
            operation.add_argument('--node-id', required=True, help='Full node ID or short reference from read.')
            operation.add_argument('--text', required=True, help='New plain-text value for the node.')
    mcp = sub.add_parser('mcp', help='Run the local stdio MCP server.')
    mcp.add_argument('--root', required=True, help='Allowed document workspace.')
    config = sub.add_parser('mcp-config', help='Print MCP client setup; does not run clients or edit configuration.')
    config.add_argument('--client', required=True, choices=['claude-code', 'codex', 'claude-desktop', 'json'],
                        help='Client setup format to print.')
    config.add_argument('--root', required=True, help='Existing document folder; home and / are refused.')
    review = sub.add_parser('open', help='Open a native document in the desktop app for visual review.')
    review.add_argument('path', help='Existing regular .hwp/.hwpx file inside the workspace.')
    review.add_argument('--root', default='.', help='Allowed workspace (default: current directory).')
    rpc = sub.add_parser('rpc', help='Run one desktop JSON request from stdin.')
    rpc.add_argument('--root', required=True, help='Allowed document workspace; desktop adapter only.')
    skill = sub.add_parser('skill', help='Install, inspect or remove the Claude Code and Codex skill.')
    actions = skill.add_subparsers(dest='skill_command', required=True, help='Skill management operation.')
    install = actions.add_parser('install', help='Install the marked skill into selected agent folders.')
    install.add_argument('--target', choices=['claude', 'codex', 'all'], default='all', help='Agent target (default: all).')
    install.add_argument('--dry-run', action='store_true', help='Validate and report destinations without writing.')
    actions.add_parser('status', help='Show installed skill versions and paths as JSON.')
    uninstall = actions.add_parser('uninstall', help='Remove only folders marked as installed by this application.')
    uninstall.add_argument('--target', choices=['claude', 'codex', 'all'], default='all', help='Agent target (default: all).')
    return parser


def _fields(values: list[str]) -> dict:
    fields = {}
    for entry in values:
        label, separator, value = entry.partition('=')
        require(bool(separator) and bool(label.strip()), 'BAD_ARGUMENTS', 'Each field requires LABEL=VALUE.')
        require(label not in fields, 'BAD_ARGUMENTS', 'Duplicate field label: ' + label)
        fields[label] = value
    return fields


def _payload(args) -> dict:
    payload = {'path': args.path}
    if args.command in {'plan', 'apply', 'propose'}:
        payload['patch'] = json.loads(Workspace(args.root).read(args.patch))
    if args.command in {'apply', 'replace', 'fill', 'set'} and args.output is not None:
        payload.update(output=args.output, acknowledgeRisks=args.acknowledge_risks)
    if args.command == 'convert':
        payload['to'] = args.to
        payload['acknowledgeRisks'] = args.acknowledge_risks
        if args.output is not None:
            payload['output'] = args.output
    if args.command == 'propose':
        payload.update(title=args.title, author=args.author)
        if args.notes is not None:
            payload['notes'] = args.notes
    if args.command == 'inspect':
        payload['compact'] = args.compact
    if args.command == 'read':
        payload.update(offset=args.offset, limit=args.limit)
    if args.command in {'search', 'replace'}:
        payload.update(regex=args.regex, caseSensitive=args.case_sensitive, limit=args.limit)
    if args.command == 'search':
        require((args.query is None) != (args.query_text is None), 'BAD_ARGUMENTS',
                'Supply a positional query or --query, but not both.')
        payload['query'] = args.query if args.query is not None else args.query_text
    if args.command == 'replace':
        payload.update(find=args.find, replaceWith=args.replace_with)
        if args.node_id is not None:
            payload['nodeIds'] = args.node_id
    if args.command == 'fill':
        payload['fields'] = json.loads(Workspace(args.root).read(args.json)) if args.json else _fields(args.field)
    if args.command == 'set':
        payload.update(nodeId=args.node_id, text=args.text)
    return payload


def main(argv=None):
    try:
        args = _parser().parse_args(argv)
        if args.command == 'mcp':
            Workspace(args.root)
            serve(args.root)
            return 0
        if args.command == 'mcp-config':
            print(mcp_config.setup(args.client, args.root))
            return 0
        if args.command == 'capabilities':
            result = CAPABILITIES
        elif args.command == 'open':
            result = desktop_open.open_document(args.root, args.path)
        elif args.command == 'skill':
            result = skill_install.run(args.skill_command, target=getattr(args, 'target', 'all'),
                                       dry_run=getattr(args, 'dry_run', False))
        elif args.command == 'rpc':
            raw = sys.stdin.read(4 * 1024 * 1024 + 1)
            require(len(raw) <= 4 * 1024 * 1024, 'LIMIT', 'Request exceeds limit.')
            request = json.loads(raw)
            require(isinstance(request, dict), 'BAD_ARGUMENTS', 'RPC request must be an object.')
            result = invoke(args.root, request['method'], request.get('args', {}))
        else:
            result = invoke(args.root, args.command, _payload(args))
        if args.command in {'extract', 'read'} and args.format == 'text':
            print(result['text'])
            if args.command == 'read':
                next_offset = json.dumps(result['nextOffset'])
                print(f"sha256={result['sha256']} nextOffset={next_offset}")
        else:
            print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0
    except (WorkbenchError, OSError, ValueError, KeyError, TypeError) as error:
        detail = error_details(error)
        print(json.dumps({'error': detail}, ensure_ascii=False), file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
