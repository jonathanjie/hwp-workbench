#!/usr/bin/env python3
"""Export self-authored test builder calls; never fetch third-party documents.

Capture runs the existing tests with real LibreOffice checks disabled. A fixed
opaque byte pattern replaces the one random test payload. The manifest stores
rebuildable arguments, including compressed base64 for large byte parameters.
"""
import argparse
import base64
from contextlib import ExitStack
import functools
import hashlib
import importlib
import inspect
import io
import json
import os
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zlib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / 'engine'), str(ROOT / 'tests')]
BUILDERS = {'fixtures': ['make_hwp', 'make_hwpx', 'cfb_from_streams'],
            'convert_fixtures': ['docx', 'package']}


def digest(payload):
    return hashlib.sha256(payload).hexdigest()


def encode(value):
    if isinstance(value, bytes):
        return {'$bytesZlibBase64': base64.b64encode(zlib.compress(value, 9)).decode('ascii')}
    if isinstance(value, dict):
        return {key: encode(item) for key, item in value.items()}
    return value


def decode(value):
    if isinstance(value, dict):
        if set(value) == {'$bytesZlibBase64'}:
            return zlib.decompress(base64.b64decode(value['$bytesZlibBase64'], validate=True))
        return {key: decode(item) for key, item in value.items()}
    return value


def rebuild(entry):
    module_name, name = entry['builder'].split('.')
    if name not in BUILDERS.get(module_name, []):
        raise ValueError('Unknown fixture builder: ' + entry['builder'])
    builder = getattr(importlib.import_module(module_name), name)
    return builder(**decode(entry['parameters']))


def test_cases(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from test_cases(item)
        else:
            yield item


class CaptureResult(unittest.TextTestResult):
    current_test = 'builder default'

    def startTest(self, test):
        self.current_test = test.id()
        super().startTest(test)


def capture():
    entries = {}
    result_holder = []

    def wrap(builder, qualified_name):
        signature = inspect.signature(builder)
        @functools.wraps(builder)
        def captured(*args, **kwargs):
            bound = signature.bind(*args, **kwargs)
            bound.apply_defaults()
            parameters = encode(bound.arguments)
            # JSON object insertion order is meaningful for stream/package maps.
            identity = qualified_name + '\n' + json.dumps(parameters, ensure_ascii=False)
            key = digest(identity.encode())[:16]
            payload = builder(*args, **kwargs)
            purpose = result_holder[0].current_test if result_holder else 'builder default'
            if key in entries:
                if entries[key]['payload'] != payload:
                    raise ValueError('Nondeterministic builder: ' + qualified_name)
                entries[key]['tests'].add(purpose)
            else:
                extension = 'hwp' if qualified_name.startswith('fixtures.') and 'hwpx' not in qualified_name else 'docx'
                if qualified_name == 'fixtures.make_hwpx':
                    extension = 'hwpx'
                elif qualified_name == 'convert_fixtures.package':
                    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
                        if 'mimetype' in archive.namelist() and archive.read('mimetype') == b'application/hwp+zip':
                            extension = 'hwpx'
                name = qualified_name.replace('.', '-') + '-' + key + '.' + extension
                entries[key] = {'name': name, 'builder': qualified_name, 'parameters': parameters,
                                'sha256': digest(payload), 'payload': payload, 'tests': {purpose}}
            return payload
        return captured

    with ExitStack() as stack:
        stack.enter_context(patch.dict(os.environ, {'HWP_WORKBENCH_LIBREOFFICE_TESTS': '0'}))
        # This test only needs unknown bytes, not cryptographic randomness.
        for module_name, names in BUILDERS.items():
            module = importlib.import_module(module_name)
            for name in names:
                stack.enter_context(patch.object(module, name, wrap(getattr(module, name), module_name + '.' + name)))
        suite = unittest.defaultTestLoader.discover(str(ROOT / 'tests'))
        stack.enter_context(patch.object(sys.modules['test_engine'], 'os', SimpleNamespace(
            urandom=lambda size: (bytes(range(256)) * ((size + 255) // 256))[:size])))
        # The rebuild/harness tests must not feed their own manifest back into capture.
        selected = unittest.TestSuite(case for case in test_cases(suite) if not case.id().startswith('test_differential.'))
        def result_class(*args, **kwargs):
            result = CaptureResult(*args, **kwargs)
            result_holder.append(result)
            return result
        result = unittest.TextTestRunner(verbosity=1, resultclass=result_class).run(selected)
        if not result.wasSuccessful():
            raise RuntimeError('Fixture capture tests failed; export was not published.')
    manifest = []
    payloads = {}
    for entry in sorted(entries.values(), key=lambda item: item['name']):
        payloads[entry['name']] = entry.pop('payload')
        entry['purpose'] = 'Exercise ' + ', '.join(sorted(entry.pop('tests')))
        # Verify outside mocks before trusting captured bytes.
        if rebuild(entry) != payloads[entry['name']]:
            raise ValueError('Captured builder depended on test mocks: ' + entry['name'])
        manifest.append(entry)
    return manifest, payloads, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Capture again and check files without writing.')
    args = parser.parse_args()
    manifest, payloads, result = capture()
    directory = ROOT / 'tests/data'
    manifest_bytes = (json.dumps(manifest, ensure_ascii=False, indent=2) + '\n').encode()
    files = payloads | {'manifest.json': manifest_bytes}
    if args.check:
        changed = [name for name, payload in files.items() if not (directory / name).exists() or (directory / name).read_bytes() != payload]
        previous = json.loads((directory / 'manifest.json').read_text())
        changed.extend(item['name'] for item in previous if item['name'] not in payloads)
        if changed:
            print('Fixture drift: ' + ', '.join(changed), file=sys.stderr)
            return 1
    else:
        directory.mkdir(parents=True, exist_ok=True)
        manifest_path = directory / 'manifest.json'
        previous = json.loads(manifest_path.read_text()) if manifest_path.exists() else []
        for name, payload in files.items():
            (directory / name).write_bytes(payload)
        for entry in previous:
            name = entry['name']
            if Path(name).name != name:
                raise ValueError('Unsafe previous manifest filename: ' + name)
            if name not in payloads:
                (directory / name).unlink(missing_ok=True)
    print(f'{len(manifest)} fixtures; {result.testsRun} capture tests; {len(result.skipped)} explicitly skipped tests.')
    for test, reason in result.skipped:
        print('SKIP ' + test.id() + ': ' + reason)
    return 0


if __name__ == '__main__':
    sys.exit(main())
