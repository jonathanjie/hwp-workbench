#!/usr/bin/env python3
"""Compare sequenced RPC, MCP and file cases in isolated workspaces.

Commands are argv strings parsed with shlex, never shell programs. References
use {"$ref": "step-id/stdout/patch"} (JSON Pointer escaping within components).
See docs/TESTING.md for the case format and exact normalisation boundaries.
"""
import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import re
import selectors
import shlex
import stat
import subprocess
import sys
import tempfile
import zipfile
import zlib

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'engine'))
from hwp_workbench.cfb import CompoundFile, MAGIC
from hwp_workbench.errors import WorkbenchError, require, MAX_PART, MAX_EXPANDED
from hwp_workbench.document import Document
from hwp_workbench.package_io import read_zip_member

DEFAULT_REFERENCE = 'python3 -m hwp_workbench'
TIMEOUT = 30  # Bound each RPC request and each MCP response, in seconds.
TIMESTAMP_KEYS = {'createdAt', 'updatedAt', 'decidedAt'}
PATH_KEYS = {'path', 'output', 'backup', 'proposal', 'file', 'name', 'target'}
HASH_KEYS = {'sha256', 'outputSha256', 'previousSha256', 'baseSha256', 'proposalSha256'}
STAMP = r'[0-9]{8}T[0-9]{6}Z'
PROPOSAL_NAME = re.compile(r'^(.*\.(?:hwp|hwpx))\.(' + STAMP + r')-([a-f0-9]{6})\.json$')
BACKUP_NAME = re.compile(r'^(.*)\.([a-f0-9]{8})\.(' + STAMP + r')-([a-f0-9]{6})(\.(?:hwp|hwpx))$')


class CaseError(Exception):
    pass


class SkipCase(Exception):
    pass


def sha(payload):
    return hashlib.sha256(payload).hexdigest()


def pointer(value, path):
    for part in path.split('/') if path else []:
        part = part.replace('~1', '/').replace('~0', '~')
        value = value[int(part)] if isinstance(value, list) else value[part]
    return value


def resolve(value, results, root):
    if isinstance(value, dict):
        if set(value) == {'$ref'}:
            return pointer(results, value['$ref'])
        if set(value) == {'$workspace'} and value['$workspace'] is True:
            return str(root)
        return {key: resolve(item, results, root) for key, item in value.items()}
    if isinstance(value, list):
        return [resolve(item, results, root) for item in value]
    return value


def zip_parts(payload):
    parts = []
    expanded = 0
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        require(len(archive.infolist()) <= 2048, 'LIMIT', 'ZIP member count exceeds 2048.')
        for item in archive.infolist():
            content = read_zip_member(archive, item, expanded, part_limit=MAX_PART, total_limit=MAX_EXPANDED)
            expanded += len(content)
            parts.append((item.filename, item.compress_type, content))
    return parts


def cfb_parts(payload):
    streams = CompoundFile(payload).streams
    header = streams.get('FileHeader', b'')
    # Reuse the frozen oracle's bounded deflate/trailer checks, without parsing
    # or executing scripts. The comparator does not edit a document.
    decoder = Document.__new__(Document)
    decoder.parts = streams
    decoder.compressed = len(header) >= 40 and bool(int.from_bytes(header[36:40], 'little') & 1)
    decoder.stream_trailers = {}
    parts = []
    expanded = 0
    for name, raw in sorted(streams.items()):
        decoded = decoder._hwp_stream(name) if name == 'DocInfo' or name.startswith(('BodyText/', 'Scripts/')) else raw
        expanded += len(decoded)
        require(expanded <= MAX_EXPANDED, 'LIMIT', 'Expanded CFB streams exceed limit.')
        parts.append((name, raw, decoded))
    return parts


def document_blobs(payload):
    """Return raw/semantic part pairs for digest checks; never execute document data."""
    if payload.startswith(b'PK'):
        return [(content, content) for _, _, content in zip_parts(payload)]
    if payload.startswith(MAGIC):
        return [(raw, decoded) for _, raw, decoded in cfb_parts(payload)]
    return []


def raw_value(payload):
    return {'container': 'bytes', 'bytesZlibBase64': base64.b64encode(zlib.compress(payload)).decode('ascii')}


def document_value(payload):
    """Keep ordered ZIP members/methods and decoded CFB streams; opaque bytes stay exact."""
    if payload.startswith(b'PK'):
        return {'container': 'zip', 'entries': [
            {'name': name, 'compression': compression, 'bytesBase64': base64.b64encode(content).decode('ascii')}
            for name, compression, content in zip_parts(payload)]}
    if payload.startswith(MAGIC):
        return {'container': 'cfb', 'streams': {
            name: base64.b64encode(decoded).decode('ascii') for name, _, decoded in cfb_parts(payload)}}
    return raw_value(payload)


def file_value(path):
    info = path.lstat()
    if stat.S_ISLNK(info.st_mode):
        return {'kind': 'symlink', 'target': os.readlink(path)}
    if stat.S_ISDIR(info.st_mode):
        return {'kind': 'directory'}
    payload = path.read_bytes()
    if path.suffix == '.json':
        try:
            content = json.loads(payload)
        except (ValueError, UnicodeError):
            content = document_value(payload)
    else:
        try:
            content = document_value(payload)
        except (ValueError, zipfile.BadZipFile, zlib.error, WorkbenchError):
            content = raw_value(payload)
    return {'kind': 'file', 'mode': stat.S_IMODE(info.st_mode), 'content': content}


def inventory(root):
    values = {}
    def visit(directory):
        for path in sorted(directory.iterdir()):
            relative = path.relative_to(root).as_posix()
            values[relative] = file_value(path)
            if path.is_dir() and not path.is_symlink():
                visit(path)
    visit(root)
    return values


class Normalizer:
    def __init__(self, root, initial_hashes):
        self.root = root
        self.initial_hashes = set(initial_hashes.values() if isinstance(initial_hashes, dict) else initial_hashes)
        self.produced_hashes = {}
        self.paths = {}
        self.generated_names = {}
        self.name_counts = {}
        self.backups = set()

    def register_bytes(self, payload, content):
        digest = sha(payload)
        if digest not in self.initial_hashes:
            identity = sha(json.dumps(self.value(content), sort_keys=True, ensure_ascii=False).encode())
            self.produced_hashes[digest] = '<produced:' + identity + '>'

    def observe(self, value, key=None):
        if isinstance(value, dict):
            if isinstance(value.get('bytesBase64'), str):
                payload = base64.b64decode(value['bytesBase64'], validate=True)
                self.register_bytes(payload, document_value(payload))
                self.register_parts(payload)
            for name, item in value.items():
                self.observe(item, name)
        elif isinstance(value, list):
            for item in value:
                self.observe(item, key)
        elif isinstance(value, str) and key in PATH_KEYS:
            self.path(value)
            if key == 'backup':
                self.backups.add(value)

    def register_parts(self, payload):
        try:
            pairs = document_blobs(payload)
        except (ValueError, zipfile.BadZipFile, zlib.error, WorkbenchError):
            return
        for raw, decoded in pairs:
            self.register_bytes(raw, raw_value(decoded))

    def register(self, files):
        for path, value in files.items():
            self.paths[path] = self.path(path)
            if value['kind'] != 'file':
                continue
            payload = (self.root / path).read_bytes()
            self.register_bytes(payload, value['content'])
            self.register_parts(payload)
            if path in self.backups:
                value['retainedByteSha256'] = sha(payload)

    def path(self, value):
        value = value.replace(str(self.root), '<workspace>')
        parts = []
        for part in value.split('/'):
            proposal = PROPOSAL_NAME.fullmatch(part)
            backup = BACKUP_NAME.fullmatch(part)
            shape = None
            if proposal:
                shape = proposal[1] + '.<timestamp>-<random>{index}.json'
            elif backup:
                # The eight-byte identity derives from the temporary canonical source path.
                shape = backup[1] + '.<workspace-identity>.<timestamp>-<random>{index}' + backup[5]
            if shape:
                if part not in self.generated_names:
                    index = self.name_counts.get(shape, 0) + 1
                    self.name_counts[shape] = index
                    self.generated_names[part] = shape.format(index=index)
                part = self.generated_names[part]
            parts.append(part)
        return '/'.join(parts)

    def value(self, value, key=None):
        if isinstance(value, dict):
            if value.get('container') in {'zip', 'cfb', 'bytes'}:
                return value
            result = {}
            for name, item in value.items():
                if name == 'bytesBase64' and isinstance(item, str):
                    # RPC preview bytes must be checked, not discarded with their digest.
                    result[name] = document_value(base64.b64decode(item, validate=True)) if key != 'entries' else item
                elif name == 'text' and value.get('type') == 'text':
                    # MCP text mirrors structuredContent; compare both after parsing JSON.
                    try:
                        result[name] = self.value(json.loads(item))
                    except (ValueError, TypeError):
                        result[name] = item
                else:
                    result[name] = self.value(item, name)
            return result
        if isinstance(value, list):
            items = [self.value(item, key) for item in value]
            if key == 'listing':
                return sorted(items)
            if key == 'proposals' and all(isinstance(item, dict) and 'file' in item for item in items):
                return sorted(items, key=lambda item: item['file'])
            return items
        if isinstance(value, str):
            if key in TIMESTAMP_KEYS and re.fullmatch(r'\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ', value):
                return '<timestamp>'
            if key in HASH_KEYS:
                return self.produced_hashes.get(value, value)
            if key in PATH_KEYS or key == 'listing':
                return self.path(value)
            if key == 'message':
                # Own literal error text stays exact; only the isolated root can vary.
                return value.replace(str(self.root), '<workspace>')
        return value

    def output(self, results, files):
        self.observe(results)
        self.register(files)
        normal_files = {}
        for path, content in files.items():
            name = self.path(path)
            if name in normal_files:
                # Do not merge two proposals/backups that happen to share a volatile shape.
                raise CaseError('Normalised filename collision: ' + name)
            normal_files[name] = self.value(content)
        return {'steps': self.value(results), 'files': normal_files}


def differences(reference, candidate, path=''):
    if type(reference) is not type(candidate):
        return [{'path': path, 'reference': reference, 'candidate': candidate}]
    if isinstance(reference, dict):
        result = []
        for key in sorted(set(reference) | set(candidate)):
            child = path + '/' + key.replace('~', '~0').replace('/', '~1')
            if key not in reference or key not in candidate:
                result.append({'path': child, 'reference': reference.get(key, '<missing>'),
                               'candidate': candidate.get(key, '<missing>')})
            else:
                result.extend(differences(reference[key], candidate[key], child))
        return result
    if isinstance(reference, list):
        result = []
        for index in range(max(len(reference), len(candidate))):
            child = path + '/' + str(index)
            if index >= len(reference) or index >= len(candidate):
                result.append({'path': child, 'reference': reference[index] if index < len(reference) else '<missing>',
                               'candidate': candidate[index] if index < len(candidate) else '<missing>'})
            else:
                result.extend(differences(reference[index], candidate[index], child))
        return result
    return [] if reference == candidate else [{'path': path, 'reference': reference, 'candidate': candidate}]


def is_approved(case, path, approvals):
    return any(item.get('case') == case and item.get('path') == path
               and isinstance(item.get('reason'), str) and item['reason'].strip()
               and isinstance(item.get('approvedBy'), str) and item['approvedBy'].strip()
               for item in approvals)


def local_path(root, name):
    path = Path(name)
    if path.is_absolute() or '..' in path.parts or not path.parts:
        raise CaseError('Case file path must be relative and confined: ' + str(name))
    target = root / path
    # Setup/file checks may inspect symlink leaves, but never traverse a link.
    if any(parent.is_symlink() for parent in target.parents if parent != root.parent):
        raise CaseError('Case file path crosses a symlink: ' + str(name))
    return target


def seed_workspace(case, root):
    hashes = {}
    for seed in case.get('seeds', []):
        source = local_path(ROOT, seed['source'])
        if not source.is_file():
            if seed.get('optional') and seed['source'].startswith('fixtures/public/'):
                raise SkipCase('Public input is absent: ' + seed['source'])
            raise CaseError('Required input is absent: ' + seed['source'])
        if not seed['source'].startswith(('tests/data/', 'fixtures/generated/', 'fixtures/public/')):
            raise CaseError('Input is outside fixture folders: ' + seed['source'])
        payload = source.read_bytes()
        target = local_path(root, seed['path'])
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(payload)
        target.chmod(0o600)
        hashes[seed['path']] = sha(payload)
    for item in case.get('setup', []):
        setup(root, item)
    return hashes


def setup(root, item):
    path = local_path(root, item['path'])
    path.parent.mkdir(parents=True, exist_ok=True)
    if item['kind'] == 'mkdir':
        path.mkdir(exist_ok=True)
    elif item['kind'] == 'symlink':
        path.symlink_to(local_path(root, item['target']))
    elif item['kind'] == 'sparse':
        with path.open('wb') as output:
            output.truncate(item['size'])
    elif item['kind'] == 'write':
        payload = json.dumps(item['json'], ensure_ascii=False).encode() if 'json' in item else item['text'].encode()
        path.write_bytes(payload)
        path.chmod(0o600)
    else:
        raise CaseError('Unknown setup kind: ' + item['kind'])


def environment():
    env = os.environ.copy()
    env['PYTHONPATH'] = str(ROOT / 'engine') + os.pathsep + env.get('PYTHONPATH', '')
    return env


def parse_json(text, channel):
    if not text.strip():
        return None
    try:
        return json.loads(text)
    except ValueError as error:
        raise CaseError('Non-JSON ' + channel + ': ' + text[:200]) from error


def rpc(command, root, request):
    result = subprocess.run(command + ['rpc', '--root', str(root)],
                            input=json.dumps(request, ensure_ascii=False), text=True,
                            capture_output=True, timeout=TIMEOUT, env=environment(), cwd=ROOT)
    return {'exitCode': result.returncode, 'stdout': parse_json(result.stdout, 'RPC stdout'),
            'stderr': parse_json(result.stderr, 'RPC stderr')}


class McpClient:
    def __init__(self, command, root):
        self.process = subprocess.Popen(command + ['mcp', '--root', str(root)],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                        text=True, bufsize=1, env=environment(), cwd=ROOT)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)

    def exchange(self, message):
        self.process.stdin.write(json.dumps(message, ensure_ascii=False) + '\n')
        self.process.stdin.flush()
        if 'id' not in message:
            return {'notification': True}
        if not self.selector.select(TIMEOUT):
            raise CaseError('MCP response timed out.')
        line = self.process.stdout.readline()
        if not line:
            raise CaseError('MCP exited before responding.')
        result = parse_json(line, 'MCP stdout')
        if not isinstance(result, dict) or result.get('jsonrpc') != '2.0' or result.get('id') != message['id']:
            raise CaseError('MCP response has an invalid envelope or ID.')
        return result

    def close(self):
        self.selector.close()
        self.process.stdin.close()
        self.process.stdin = None
        try:
            stdout, stderr = self.process.communicate(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.communicate()
            raise CaseError('MCP did not exit after stdin closed.')
        if self.process.returncode != 0 or stdout.strip() or stderr.strip():
            raise CaseError(f'MCP shutdown exit={self.process.returncode}, stdout={stdout[:200]!r}, stderr={stderr[:200]!r}')


def check_expected(step, result):
    for path, expected in step.get('expect', {}).items():
        try:
            observed = pointer(result, path)
        except (KeyError, IndexError, TypeError, ValueError) as error:
            raise CaseError(step['id'] + ': expected result path is absent: ' + path) from error
        if type(observed) is not type(expected) or observed != expected:
            raise CaseError(f'{step["id"]}: expected {path}={expected!r}, received {observed!r}')


def verify_output_digests(body, root):
    if 'bytesBase64' in body and 'sha256' in body:
        payload = base64.b64decode(body['bytesBase64'], validate=True)
        if body['sha256'] != sha(payload):
            raise CaseError('Preview digest does not match its bytes.')
    checks = []
    if 'outputSha256' in body and 'output' in body:
        checks.append(('outputSha256', body['output']))
    if body.get('saved') is True and 'sha256' in body:
        checks.append(('sha256', body['path']))
    for key, name in checks:
        path = local_path(root, name)
        if path.is_symlink() or not path.is_file() or body[key] != sha(path.read_bytes()):
            raise CaseError(key + ' digest does not match the produced file: ' + name)


def execute(case, command):
    with tempfile.TemporaryDirectory(prefix='hwp-differential-') as folder:
        root = Path(folder).resolve()
        initial_hashes = seed_workspace(case, root)
        results = {}
        mutable_inputs = set()
        normal = Normalizer(root, initial_hashes)
        for path in initial_hashes:
            try:
                blobs = document_blobs((root / path).read_bytes())
            except (ValueError, zipfile.BadZipFile, zlib.error, WorkbenchError):
                continue
            normal.initial_hashes.update(sha(raw) for raw, _ in blobs)
        mcp = None
        try:
            for step in case['steps']:
                value = resolve(step, results, root)
                kind = value['kind']
                if kind == 'rpc':
                    result = rpc(command, root, {'method': value['method'], 'args': value.get('args', {})})
                    if value.get('skipIfUnavailable') and result['exitCode'] == 0 and isinstance(result['stdout'], dict) and result['stdout'].get('available') is False:
                        raise SkipCase('Optional conversion backend unavailable: ' + json.dumps(result['stdout']['engineReports']))
                elif kind == 'mcp':
                    if mcp is None:
                        mcp = McpClient(command, root)
                    result = mcp.exchange(value['message'])
                elif kind == 'list':
                    path = local_path(root, value['path'])
                    result = {'listing': sorted(item.name for item in path.iterdir()) if path.is_dir() else [],
                              'exists': path.exists()}
                elif kind == 'file':
                    path = local_path(root, value['path'])
                    result = {'exists': path.exists(), 'value': file_value(path) if path.exists() else None}
                elif kind in {'write', 'mkdir', 'symlink', 'sparse'}:
                    setup(root, value)
                    result = {'prepared': True}
                else:
                    raise CaseError('Unknown case step kind: ' + kind)
                check_expected(value, result)
                if kind == 'write' or (kind == 'rpc' and value['method'] == 'save' and result['exitCode'] == 0):
                    mutable_inputs.add(value['path'] if kind == 'write' else value['args']['path'])
                results[step['id']] = result
                # Keep digests for intermediate saves that a later save can replace.
                body = result.get('stdout') if kind == 'rpc' else None
                if kind == 'mcp' and isinstance(result.get('result'), dict):
                    body = result['result'].get('structuredContent')
                if isinstance(body, dict):
                    verify_output_digests(body, root)
                    produced = [body[name] for name in ['output', 'proposal', 'backup'] if isinstance(body.get(name), str)]
                    if body.get('saved') is True:
                        produced.append(body['path'])
                    for name in produced:
                        target = local_path(root, name)
                        if target.is_file() and not target.is_symlink():
                            content = file_value(target)['content']
                            payload = target.read_bytes()
                            normal.register_bytes(payload, content)
                            normal.register_parts(payload)
                normal.observe(result)
            for path, expected_hash in initial_hashes.items():
                if path not in mutable_inputs:
                    source = root / path
                    if source.is_symlink() or not source.is_file() or sha(source.read_bytes()) != expected_hash:
                        raise CaseError('Source bytes changed without save: ' + path)
            files = inventory(root)
            return normal.output(results, files)
        finally:
            if mcp is not None:
                mcp.close()


def validate_cases(cases):
    ids = set()
    for case in cases:
        if not isinstance(case.get('id'), str) or case['id'] in ids or not case.get('purpose'):
            raise CaseError('Cases require a unique id and a purpose.')
        ids.add(case['id'])
        step_ids = set()
        if not case.get('steps'):
            raise CaseError('Case has no steps: ' + case['id'])
        for step in case['steps']:
            if not step.get('id') or step['id'] in step_ids:
                raise CaseError('Steps require unique ids: ' + case['id'])
            step_ids.add(step['id'])
            if step.get('kind') not in {'rpc', 'mcp', 'file', 'list', 'write', 'mkdir', 'sparse', 'symlink'}:
                raise CaseError('Unknown step kind: ' + str(step.get('kind')))


def short(value):
    text = json.dumps(value, ensure_ascii=False)
    return text if len(text) <= 160 else text[:157] + '...'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', default=DEFAULT_REFERENCE)
    parser.add_argument('--candidate', required=True)
    parser.add_argument('--cases', type=Path, default=ROOT / 'tests/contract/cases.json')
    parser.add_argument('--only', help='Run one stable case id.')
    parser.add_argument('--report', type=Path, help='Write complete differences and host evidence as JSON.')
    args = parser.parse_args()
    cases = json.loads(args.cases.read_text())
    validate_cases(cases)
    if args.only:
        cases = [case for case in cases if case['id'] == args.only]
        if not cases:
            parser.error('Unknown case id: ' + args.only)
    commands = [shlex.split(args.reference), shlex.split(args.candidate)]
    if not all(commands):
        parser.error('Commands must not be empty.')
    approvals = json.loads((ROOT / 'tests/contract/approved-differences.json').read_text())
    results = []
    for case in cases:
        record = {'case': case['id'], 'purpose': case['purpose']}
        try:
            reference = execute(case, commands[0])
        except SkipCase as error:
            record.update(status='SKIP', reason=str(error))
        except (CaseError, OSError, ValueError, KeyError, IndexError, TypeError, subprocess.TimeoutExpired) as error:
            record.update(status='DIFF', reason='Reference failed case contract: ' + str(error))
        else:
            try:
                candidate = execute(case, commands[1])
                changes = differences(reference, candidate)
                unapproved = [item for item in changes if not is_approved(case['id'], item['path'], approvals)]
                record.update(status='DIFF' if unapproved else 'PASS', differences=changes,
                              approved=len(changes) - len(unapproved))
            except (CaseError, SkipCase, OSError, ValueError, KeyError, IndexError, TypeError, subprocess.TimeoutExpired) as error:
                record.update(status='DIFF', reason='Candidate failed case contract: ' + str(error))
        results.append(record)
        details = record.get('reason', '')
        if record.get('differences'):
            details = '; '.join(item['path'] + ': ' + short(item['reference']) + ' != ' + short(item['candidate'])
                                for item in record['differences'][:3])
            if len(record['differences']) > 3:
                details += f'; {len(record["differences"])} differences total (use --report)'
        print(record['status'] + ' ' + case['id'] + (' — ' + details if details else ''), flush=True)
    summary = {status: sum(item['status'] == status for item in results) for status in ['PASS', 'DIFF', 'SKIP']}
    print('Summary: ' + ', '.join(f'{count} {status}' for status, count in summary.items()))
    if args.report:
        import platform
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps({'reference': args.reference, 'candidate': args.candidate,
                                          'platform': platform.platform(), 'python': platform.python_version(),
                                          'summary': summary, 'cases': results}, ensure_ascii=False, indent=2) + '\n')
    return 1 if summary['DIFF'] else 0


if __name__ == '__main__':
    sys.exit(main())
