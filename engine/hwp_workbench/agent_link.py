"""Bounded proposal files shared by agents and the desktop's Agent link."""
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import secrets

from .document import Document, sha, patch_input
from .errors import WorkbenchError, require
from .workspace import Workspace

MAX_LINK_FILE = 2 * 1024 * 1024
MAX_PROPOSALS = 200
MISSING_FILE = object()


def _utc_now() -> str:
    return datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')


def _link_folder(workspace: Workspace, path: str) -> Path:
    return Path(workspace.relative_path(path)).parent / '.hwp-workbench'


def propose(workspace: Workspace, path: str, document: Document, args: dict) -> dict:
    title = args.get('title')
    author = args.get('author', 'agent')
    notes = args.get('notes')
    require(isinstance(title, str) and bool(title.strip()), 'BAD_ARGUMENTS', 'Supply a nonempty proposal title.')
    require(isinstance(author, str) and bool(author.strip()), 'BAD_ARGUMENTS', 'Supply a nonempty proposal author.')
    require('notes' not in args or isinstance(notes, str), 'BAD_ARGUMENTS', 'Proposal notes must be text.')
    patch = patch_input(args.get('patch'))
    document.patch(patch)
    # Resolve aliases now so the desktop always receives native IDs.
    native_patch = dict(patch)
    native_patch['operations'] = [operation | {'nodeId': document.resolve_node_id(operation['nodeId'])}
                                  for operation in patch['operations']]
    created_at = _utc_now()
    proposal = {
        'schemaVersion': 1,
        'kind': 'hwp-workbench-proposal',
        'document': {'name': document.name, 'sha256': document.hash},
        'title': title,
        'author': author,
        'createdAt': created_at,
        'patch': native_patch,
    }
    if notes is not None:
        proposal['notes'] = notes
    content = (json.dumps(proposal, ensure_ascii=False, indent=2) + '\n').encode('utf-8')
    require(len(content) <= MAX_LINK_FILE, 'LIMIT', 'Proposal exceeds 2 MiB.')
    require(sha(workspace.read(path)) == document.hash, 'STALE_DOCUMENT',
            'Source changed during processing; proposal was not published.')
    folder = _link_folder(workspace, path) / 'proposals'
    workspace.ensure_directory(str(folder))
    timestamp = created_at.replace('-', '').replace(':', '')
    filename = f'{document.name}.{timestamp}-{secrets.token_hex(3)}.json'
    destination = str(folder / filename)
    workspace.write_new(destination, content)
    return {'proposal': destination, 'changes': document.diff()['changes']}


def _read_json(workspace: Workspace, path: str):
    """Return (value, readable); missing files are distinct from broken files."""
    try:
        payload = workspace.read(path, max_bytes=MAX_LINK_FILE, missing_ok=True)
        if payload is None:
            return MISSING_FILE, True
        return json.loads(payload), True
    except (WorkbenchError, ValueError, UnicodeError, RecursionError):
        return None, False


def _text(value) -> bool:
    return isinstance(value, str)


def _valid_patch(value) -> bool:
    if not isinstance(value, dict) or type(value.get('schemaVersion')) is not int or value['schemaVersion'] != 1:
        return False
    if not _text(value.get('baseSha256')):
        return False
    operations = value.get('operations')
    if not isinstance(operations, list) or not 1 <= len(operations) <= 1000:
        return False
    return all(isinstance(operation, dict)
               and set(operation) == {'op', 'nodeId', 'expectedText', 'text'}
               and operation['op'] == 'set_text'
               and all(_text(operation[key]) for key in ('nodeId', 'expectedText', 'text'))
               for operation in operations)


def _valid_session(value, document: Document) -> bool:
    if not isinstance(value, dict) or type(value.get('schemaVersion')) is not int or value['schemaVersion'] != 1:
        return False
    if value.get('app') != 'hwp-workbench' or not _text(value.get('appVersion')) or not _text(value.get('updatedAt')):
        return False
    identity = {'name': document.name, 'format': document.format, 'sha256': document.hash}
    if value.get('document') != identity or 'selection' not in value or 'staged' not in value:
        return False
    selection = value['selection']
    if selection is not None and (not isinstance(selection, dict)
                                  or not _text(selection.get('nodeId')) or not _text(selection.get('text'))):
        return False
    return value['staged'] is None or _valid_patch(value['staged'])


def _valid_proposal(value, document: Document) -> bool:
    if not isinstance(value, dict) or type(value.get('schemaVersion')) is not int or value['schemaVersion'] != 1:
        return False
    identity = value.get('document')
    if not isinstance(identity, dict) or identity.get('name') != document.name or not _text(identity.get('sha256')):
        return False
    if value.get('kind') != 'hwp-workbench-proposal':
        return False
    if not all(_text(value.get(key)) for key in ('title', 'author', 'createdAt')):
        return False
    return _valid_patch(value.get('patch')) and value['patch']['baseSha256'] == identity['sha256']


def _valid_outcome(value) -> bool:
    if not isinstance(value, dict) or type(value.get('schemaVersion')) is not int or value['schemaVersion'] != 1:
        return False
    if value.get('status') not in ('accepted', 'declined', 'partial') or not _text(value.get('decidedAt')):
        return False
    for key in ('acceptedNodeIds', 'declinedNodeIds'):
        node_ids = value.get(key)
        if not isinstance(node_ids, list) or len(node_ids) > 1000 or not all(_text(node_id) for node_id in node_ids):
            return False
    return True


def _proposal_summary(workspace: Workspace, path: str, document: Document) -> dict:
    summary = {'file': path, 'title': None, 'author': None, 'createdAt': None,
               'operations': 0, 'status': 'unreadable'}
    proposal, readable = _read_json(workspace, path)
    if not readable or not _valid_proposal(proposal, document):
        return summary
    summary.update(title=proposal['title'], author=proposal['author'], createdAt=proposal['createdAt'],
                   operations=len(proposal['patch']['operations']))
    outcome_path = str(Path(path).with_suffix('.outcome.json'))
    outcome, readable = _read_json(workspace, outcome_path)
    if not readable:
        return summary
    if outcome is not MISSING_FILE:
        if _valid_outcome(outcome):
            summary['status'] = outcome['status']
    else:
        summary['status'] = 'pending' if proposal['document']['sha256'] == document.hash else 'stale'
    return summary


def session(workspace: Workspace, path: str, document: Document) -> dict:
    folder = _link_folder(workspace, path)
    session_path = str(folder / (document.name + '.session.json'))
    record, readable = _read_json(workspace, session_path)
    linked = readable and _valid_session(record, document)
    proposal_folder = folder / 'proposals'
    name_pattern = re.compile(re.escape(document.name) + r'\.[0-9]{8}T[0-9]{6}Z-[a-f0-9]{6}\.json')
    filenames = workspace.list_files(str(proposal_folder), name_pattern=name_pattern, limit=MAX_PROPOSALS)
    proposals = [_proposal_summary(workspace, str(proposal_folder / name), document) for name in filenames]
    return {'linked': linked, 'session': record if linked else None, 'proposals': proposals}
