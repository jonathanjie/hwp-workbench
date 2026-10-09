"""Executable without third-party Python dependencies."""
import argparse
import json
import sys

from .errors import WorkbenchError, require, error_details
from .service import invoke, CAPABILITIES
from .mcp import serve
from .workspace import Workspace


class ArgumentParser(argparse.ArgumentParser):
    def error(self, message):
        raise WorkbenchError('BAD_ARGUMENTS', message)


def _parser() -> argparse.ArgumentParser:
    parser = ArgumentParser(prog='hwp-workbench', description='Local HWP/HWPX preservation editor — development alpha.')
    parser.add_argument('--version', action='version', version=CAPABILITIES['version'])
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('capabilities')
    for command in CAPABILITIES['operations']:
        operation = sub.add_parser(command)
        operation.add_argument('path')
        operation.add_argument('--root', default='.', help='Allowed workspace (default: current directory).')
        if command in {'plan', 'apply'}:
            operation.add_argument('--patch', required=True, help='JSON patch file inside workspace.')
        if command in {'apply', 'replace', 'fill', 'set'}:
            operation.add_argument('-o', '--output', required=command == 'apply')
            operation.add_argument('--acknowledge-risks', action='store_true')
        if command == 'inspect':
            operation.add_argument('--compact', action='store_true')
        if command == 'extract':
            operation.add_argument('--format', choices=['text', 'json'], default='json')
        if command == 'read':
            operation.add_argument('--offset', type=int, default=0)
            operation.add_argument('--limit', type=int, default=200)
        if command in {'search', 'replace'}:
            operation.add_argument('--regex', action='store_true')
            case = operation.add_mutually_exclusive_group()
            case.add_argument('--case-sensitive', dest='case_sensitive', action='store_true')
            case.add_argument('--ignore-case', dest='case_sensitive', action='store_false')
            operation.set_defaults(case_sensitive=command == 'replace')
            operation.add_argument('--limit', type=int, default=100 if command == 'search' else 1000)
        if command == 'search':
            operation.add_argument('--query', required=True)
        if command == 'replace':
            operation.add_argument('--find', required=True)
            operation.add_argument('--replace-with', required=True)
            operation.add_argument('--node-id', action='append', help='Full node ID; repeat to restrict targets.')
        if command == 'fill':
            operation.add_argument('--json', required=True, help='JSON label/value object file inside workspace.')
        if command == 'set':
            operation.add_argument('--node-id', required=True)
            operation.add_argument('--text', required=True)
    mcp = sub.add_parser('mcp')
    mcp.add_argument('--root', required=True)
    rpc = sub.add_parser('rpc')
    rpc.add_argument('--root', required=True, help='One JSON request on stdin; desktop adapter only.')
    return parser


def _payload(args) -> dict:
    payload = {'path': args.path}
    if args.command in {'plan', 'apply'}:
        payload['patch'] = json.loads(Workspace(args.root).read(args.patch))
    if args.command in {'apply', 'replace', 'fill', 'set'} and args.output is not None:
        payload.update(output=args.output, acknowledgeRisks=args.acknowledge_risks)
    if args.command == 'inspect':
        payload['compact'] = args.compact
    if args.command == 'read':
        payload.update(offset=args.offset, limit=args.limit)
    if args.command in {'search', 'replace'}:
        payload.update(regex=args.regex, caseSensitive=args.case_sensitive, limit=args.limit)
    if args.command == 'search':
        payload['query'] = args.query
    if args.command == 'replace':
        payload.update(find=args.find, replaceWith=args.replace_with)
        if args.node_id is not None:
            payload['nodeIds'] = args.node_id
    if args.command == 'fill':
        payload['fields'] = json.loads(Workspace(args.root).read(args.json))
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
        if args.command == 'capabilities':
            result = CAPABILITIES
        elif args.command == 'rpc':
            raw = sys.stdin.read(4 * 1024 * 1024 + 1)
            require(len(raw) <= 4 * 1024 * 1024, 'LIMIT', 'Request exceeds limit.')
            request = json.loads(raw)
            require(isinstance(request, dict), 'BAD_ARGUMENTS', 'RPC request must be an object.')
            result = invoke(args.root, request['method'], request.get('args', {}))
        else:
            result = invoke(args.root, args.command, _payload(args))
        if args.command == 'extract' and args.format == 'text':
            print(result['text'])
        else:
            print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0
    except (WorkbenchError, OSError, ValueError, KeyError, TypeError) as error:
        detail = error_details(error)
        print(json.dumps({'error': detail}, ensure_ascii=False), file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
