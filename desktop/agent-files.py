"""Fixed desktop link-file actions using directory descriptors, never path APIs."""
import json
import os
from pathlib import Path
import re
import secrets
import stat
import sys
from types import SimpleNamespace
from datetime import datetime, timezone

from hwp_workbench import agent_link
from hwp_workbench.errors import WorkbenchError, require
from hwp_workbench.workspace import Workspace


def publish_session(workspace, name, record):
    identity = record.get('document', {}) if isinstance(record, dict) else {}
    document = SimpleNamespace(name=name, format=Path(name).suffix[1:].lower(), hash=identity.get('sha256'))
    require(agent_link._valid_session(record, document), 'BAD_ARGUMENTS', 'Invalid session.')
    workspace.ensure_directory('.hwp-workbench/proposals')
    payload = (json.dumps(record, ensure_ascii=False) + '\n').encode()
    require(len(payload) <= agent_link.MAX_LINK_FILE, 'LIMIT', 'Session exceeds 2 MiB.')
    # Workspace owns traversal. Hold the parent descriptor across atomic rename.
    parent, destination = workspace._parent('.hwp-workbench/' + name + '.session.json')
    temporary = '.session-' + secrets.token_hex(12)
    try:
        try:
            info = os.stat(destination, dir_fd=parent, follow_symlinks=False)
            require(stat.S_ISREG(info.st_mode), 'UNSAFE_PATH', 'Session is nonregular or a symlink.')
        except FileNotFoundError:
            pass
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=parent)
        with os.fdopen(descriptor, 'wb') as output:
            output.write(payload)
            output.flush()
            os.fsync(output.fileno())
        os.rename(temporary, destination, src_dir_fd=parent, dst_dir_fd=parent)
    finally:
        try:
            os.unlink(temporary, dir_fd=parent)
        except FileNotFoundError:
            pass
        os.close(parent)
    return True


def proposal_pattern(name):
    return re.compile(re.escape(name) + r'\.[0-9]{8}T[0-9]{6}Z-[a-f0-9]{6}\.json')


def read_proposals(workspace, name, hash_):
    folder = '.hwp-workbench/proposals'
    filenames = workspace.list_files(folder, name_pattern=proposal_pattern(name), limit=agent_link.MAX_PROPOSALS)
    document = SimpleNamespace(name=name, hash=hash_)
    rows = []
    for filename in filenames:
        file = folder + '/' + filename
        summary = agent_link._proposal_summary(workspace, file, document)
        summary['file'] = filename
        if summary['status'] in ('pending', 'stale'):
            proposal, readable, _ = agent_link._read_json(workspace, file)
            if readable and agent_link._valid_proposal(proposal, document):
                summary['patch'] = proposal['patch']
        rows.append(summary)
    return rows


def publish_outcome(workspace, name, args):
    filename = args.get('file')
    require(isinstance(filename, str) and proposal_pattern(name).fullmatch(filename), 'BAD_ARGUMENTS', 'Invalid proposal file.')
    proposal, readable, proposal_sha256 = agent_link._read_json(workspace, '.hwp-workbench/proposals/' + filename)
    require(readable and agent_link._valid_proposal(proposal, SimpleNamespace(name=name)), 'BAD_ARGUMENTS', 'Invalid proposal.')
    if 'expectedPatch' in args:
        require(proposal['patch'] == args['expectedPatch'], 'STALE_DOCUMENT', 'Proposal changed before the outcome write.')
    accepted = args.get('acceptedNodeIds')
    declined = args.get('declinedNodeIds')
    require(isinstance(accepted, list) and isinstance(declined, list) and
            all(isinstance(value, str) for value in accepted + declined), 'BAD_ARGUMENTS', 'Invalid outcome node IDs.')
    expected = [operation['nodeId'] for operation in proposal['patch']['operations']]
    require(len(set(expected)) == len(expected) and len(set(accepted + declined)) == len(accepted + declined)
            and set(accepted + declined) == set(expected), 'BAD_ARGUMENTS', 'Outcome must contain the complete proposal once.')
    # Bind the decision to the exact proposal bytes that were reviewed, so a
    # replaced proposal with the same name is not reported as decided.
    outcome = {'schemaVersion': 1, 'status': 'partial' if accepted and declined else 'accepted' if accepted else 'declined',
               'proposalSha256': proposal_sha256,
               'acceptedNodeIds': accepted, 'declinedNodeIds': declined,
               'decidedAt': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')}
    workspace.write_new('.hwp-workbench/proposals/' + filename.removesuffix('.json') + '.outcome.json',
                        (json.dumps(outcome) + '\n').encode())
    return outcome


def main():
    root, action, name = sys.argv[1:]
    require(Path(name).name == name and Path(name).suffix.lower() in ('.hwp', '.hwpx'), 'BAD_ARGUMENTS', 'Invalid document name.')
    payload = sys.stdin.buffer.read(agent_link.MAX_LINK_FILE + 1)
    require(len(payload) <= agent_link.MAX_LINK_FILE, 'LIMIT', 'Link request exceeds 2 MiB.')
    args = json.loads(payload)
    workspace = Workspace(root)
    if action == 'read':
        return read_proposals(workspace, name, args['sha256'])
    if action == 'session':
        return publish_session(workspace, name, args)
    require(action == 'outcome', 'BAD_ARGUMENTS', 'Unknown link action.')
    return publish_outcome(workspace, name, args)


if __name__ == '__main__':
    try:
        print(json.dumps(main(), ensure_ascii=False))
    except (WorkbenchError, ValueError, KeyError, OSError) as error:
        print(json.dumps({'error': {'message': str(error)}}), file=sys.stderr)
        sys.exit(2)
