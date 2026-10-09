"""Shared document operations for CLI, MCP and desktop RPC."""
from pathlib import Path

from . import agent_ops, agent_link
from .document import Document, sha, patch_input
from .independent import check_output
from .errors import require
from .workspace import Workspace

CAPABILITIES = {
    'version': '0.1.0-alpha.1',
    'formats': {
        'hwp': 'HWP 5.x: existing plain text runs; variable length; Hancom reflow unverified',
        'hwpx': 'Existing plain text nodes; variable length; same-format save',
    },
    'operations': ['inspect', 'extract', 'validate', 'plan', 'apply',
                   'read', 'search', 'replace', 'fill', 'set', 'propose', 'session'],
    'notImplemented': ['native full-page rendering', 'formatting and structural edits',
                       'new document authoring', 'HWP/HWPX conversion', 'DOCX/PDF export',
                       'comments/track changes', 'full Hancom fidelity'],
    'safety': {'overwrite': False, 'externalNetwork': False, 'execDocumentCode': False,
               'workspaceRestricted': True, 'requireRiskAcknowledgement': True},
}


def _publish(ws: Workspace, path: str, doc: Document, args: dict, result: dict) -> dict:
    output = args.get('output')
    require(isinstance(output, str) and Path(output).suffix.lower() == '.' + doc.format,
            'OUTPUT_FORMAT', 'Output must use the same native extension as the source.')
    require(args.get('acknowledgeRisks') is True, 'RISK_ACK_REQUIRED',
            'Pass acknowledgeRisks: true to save edited output.')
    content = doc.serialize(True)
    independent_check = check_output(content, doc.edits)
    # Re-read immediately before publication. Original sources never get overwritten.
    require(sha(ws.read(path)) == doc.hash, 'STALE_DOCUMENT',
            'Source changed during processing; output was not published.')
    ws.write_new(output, content)
    return result | {'output': output, 'outputSha256': sha(content),
                     'reopenedAndVerified': True, 'sourceUntouched': True,
                     'independentCheck': independent_check}


def _agent_edit(doc: Document, method: str, args: dict) -> dict:
    if method == 'replace':
        return agent_ops.replace(doc, args.get('find'), args.get('replaceWith'),
                                 regex=args.get('regex', False),
                                 case_sensitive=args.get('caseSensitive', True),
                                 node_ids=args.get('nodeIds'), limit=args.get('limit', 1000))
    if method == 'fill':
        return agent_ops.fill(doc, args.get('fields'))
    return agent_ops.set_text(doc, args.get('nodeId'), args.get('text'))


def invoke(root, method: str, args: dict) -> dict:
    require(isinstance(args, dict), 'BAD_ARGUMENTS', 'Arguments must be an object.')
    require(isinstance(method, str), 'BAD_ARGUMENTS', 'method must be a string.')
    if method == 'capabilities':
        return CAPABILITIES
    require(method in CAPABILITIES['operations'], 'UNKNOWN_METHOD', 'Unknown operation: ' + method)
    ws = Workspace(root)
    path = args.get('path')
    doc = Document(ws.read(path), Path(path).name)
    if method == 'session':
        return agent_link.session(ws, path, doc)
    if method == 'propose':
        return agent_link.propose(ws, path, doc, args)
    if method == 'inspect':
        compact = args.get('compact', False)
        require(type(compact) is bool, 'BAD_ARGUMENTS', 'compact must be a boolean.')
        result = doc.inspect()
        if compact:
            result.pop('view')
            result.pop('parts')
        return result
    if method == 'read':
        return agent_ops.read(doc, offset=args.get('offset', 0), limit=args.get('limit', 200))
    if method == 'search':
        return agent_ops.search(doc, args.get('query'), regex=args.get('regex', False),
                                case_sensitive=args.get('caseSensitive', False), limit=args.get('limit', 100))
    if method == 'extract':
        return {'format': doc.format, 'sha256': doc.hash,
                'text': '\n'.join(node.text for node in doc.nodes), 'warnings': doc.warnings}
    if method == 'validate':
        return {'valid': True, 'scope': 'bounded container/XML/record parsing only; not Hancom rendering certification',
                'sha256': doc.hash, 'format': doc.format, 'textNodes': len(doc.nodes), 'warnings': doc.warnings}
    if method in {'replace', 'fill', 'set'}:
        result = _agent_edit(doc, method, args)
        patch = result['patch']
        if patch['operations']:
            doc.patch(patch)
        result.update(doc.diff())
        if 'output' not in args:
            return result
        # Empty previews are useful; an empty patch cannot be applied as an edit.
        require(bool(patch['operations']), 'BAD_PATCH', 'No editable changes were selected; no output was written.')
        return _publish(ws, path, doc, args, result)
    doc.patch(patch_input(args.get('patch')))
    result = doc.diff()
    if method == 'plan':
        return result
    return _publish(ws, path, doc, args, result)
