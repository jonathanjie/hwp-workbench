#!/usr/bin/env python3
"""Opt-in, headless native/WASM probe comparison and worker measurements.

Build with cargo-offline.sh first, then pass --worker /path/to/hwp-verify-worker.
No downloads, fixture generation, or office/GUI applications are used.
The JSON report is written to stdout; absent public files remain explicit skips.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import selectors
import shutil
import signal
import struct
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
MAX_DOCUMENT = 64 * 1024 * 1024  # Same document bound as the bridge.
MAX_OUTPUT = 64 * 1024 * 1024  # Combined stdout/stderr bound.
CHUNK_SIZE = 64 * 1024  # Drain both pipes while sending input.
TIMEOUT = 30  # Same per-probe deadline as the bridge, in seconds.

# A fresh interpreter has exactly one child, so RUSAGE_CHILDREN is this worker's
# high-water RSS, rather than the maximum across earlier WASM/native probes.
MEMORY_PROBE = """
import resource, subprocess, sys
status = subprocess.run([sys.argv[1]], input=sys.stdin.buffer.read(),
                        stdout=sys.stdout.buffer, stderr=sys.stderr.buffer).returncode
rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
scale = 1 if sys.platform == 'darwin' else 1024
print('HWP_VERIFY_PEAK_MEMORY_BYTES: ' + str(int(rss * scale)), file=sys.stderr)
sys.exit(status)
"""


def run_probe(command, payload, *, allow_profiler_failure=False):
    request = b'{"op":"probe"}'
    frame = struct.pack('<I', len(request)) + request + payload
    started = time.monotonic()
    deadline = started + TIMEOUT
    output = bytearray()
    stderr = bytearray()
    received = 0
    sent = 0
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    try:
        with selectors.DefaultSelector() as selector:
            for stream in (process.stdin, process.stdout, process.stderr):
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, selectors.EVENT_WRITE if stream is process.stdin else selectors.EVENT_READ)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError('rhwp exceeded the 30-second timeout.')
                for key, _ in selector.select(remaining):
                    stream = key.fileobj
                    if stream is process.stdin:
                        try:
                            sent += os.write(stream.fileno(), frame[sent:sent + CHUNK_SIZE])
                        except BrokenPipeError:
                            sent = len(frame)
                        if sent == len(frame):
                            selector.unregister(stream)
                            stream.close()
                        continue
                    chunk = os.read(stream.fileno(), CHUNK_SIZE)
                    if not chunk:
                        selector.unregister(stream)
                        continue
                    received += len(chunk)
                    if received > MAX_OUTPUT:
                        raise ValueError('rhwp output exceeds 64 MiB.')
                    (output if stream is process.stdout else stderr).extend(chunk)
            process.wait(timeout=max(0, deadline - time.monotonic()))
        observed = json.loads(output)
        if not isinstance(observed, dict):
            raise ValueError('rhwp returned a non-object response.')
        if process.returncode and 'error' not in observed and not allow_profiler_failure:
            raise ValueError(f'Worker exited {process.returncode} without an error response.')
        return observed, time.monotonic() - started, stderr.decode('utf-8', errors='replace')
    finally:
        if process.poll() is None:
            # time(1) is a wrapper: stop its worker too on any transport failure.
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        for stream in (process.stdin, process.stdout, process.stderr):
            stream.close()


def peak_memory(stderr):
    match = re.search(r'^HWP_VERIFY_PEAK_MEMORY_BYTES:\s*(\d+)\s*$', stderr, re.MULTILINE)
    if match:
        return int(match.group(1))
    if sys.platform == 'darwin':
        match = re.search(r'^\s*(\d+)\s+maximum resident set size\s*$', stderr, re.MULTILINE)
        return int(match.group(1)) if match else None
    match = re.search(r'Maximum resident set size \(kbytes\):\s*(\d+)', stderr)
    return int(match.group(1)) * 1024 if match else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker', required=True, type=Path)
    args = parser.parse_args()
    worker = args.worker.resolve(strict=True)
    node = shutil.which('node')
    if node is None:
        parser.error('Node.js is required for the WASM cross-check.')
    report = {
        'host': {'platform': platform.platform(), 'machine': platform.machine(),
                 'python': platform.python_version()},
        'worker': str(worker), 'workerBytes': worker.stat().st_size,
        'files': [], 'skipped': [], 'differences': [], 'unsupported': [],
    }
    time_program = Path('/usr/bin/time')
    native_command = [str(worker)]
    profile_command = ([str(time_program), '-l' if sys.platform == 'darwin' else '-v', str(worker)]
                       if time_program.is_file() else None)
    paths = sorted(path for folder in ('generated', 'public')
                   for path in (ROOT / 'fixtures' / folder).rglob('*')
                   if path.is_file() and path.suffix.lower() in ('.hwp', '.hwpx'))
    manifest = json.loads((ROOT / 'fixtures/public-manifest.json').read_text())
    for fixture in manifest['fixtures']:
        if not (ROOT / 'fixtures/public' / fixture['name']).is_file():
            report['skipped'].append({'file': fixture['name'], 'reason': 'Public binary fixture is absent; no network download permitted.'})
    for path in paths:
        entry = {'file': str(path.relative_to(ROOT))}
        report['files'].append(entry)
        if path.stat().st_size > MAX_DOCUMENT:
            entry['error'] = 'Input exceeds 64 MiB.'
            report['unsupported'].append(entry['file'])
            continue
        payload = path.read_bytes()
        entry['sha256'] = hashlib.sha256(payload).hexdigest()
        for engine, command in [('native', native_command), ('wasm', [node, str(ROOT / 'engine/bridge/rhwp-bridge.mjs')])]:
            try:
                observed, seconds, stderr = run_probe(command, payload)
                entry[engine] = {'seconds': round(seconds, 6), 'pageCount': observed.get('pageCount')}
                entry[engine]['probe'] = observed
            except (OSError, ValueError, TimeoutError, subprocess.TimeoutExpired) as error:
                entry[engine] = {'probe': {'error': {'code': 'CROSSCHECK_FAILED', 'message': str(error)}}}
        native = entry['native'].pop('probe')
        wasm = entry['wasm'].pop('probe')
        memory = None
        memory_method = '/usr/bin/time'
        memory_reason = '/usr/bin/time is absent.'
        if profile_command is not None:
            try:
                measured, _, stderr = run_probe(profile_command, payload, allow_profiler_failure=True)
                if measured == native:
                    memory = peak_memory(stderr)
                    memory_reason = stderr.strip()[:300] or '/usr/bin/time metric absent.'
                else:
                    memory_reason = 'Profiler probe differs from the direct native probe.'
            except (OSError, ValueError, TimeoutError, subprocess.TimeoutExpired) as error:
                memory_reason = str(error)
        if memory is None:
            report['skipped'].append({'file': entry['file'], 'reason': 'time(1) memory measurement unavailable: ' + memory_reason})
            try:
                measured, _, stderr = run_probe([sys.executable, '-c', MEMORY_PROBE, str(worker)], payload)
                if measured == native:
                    memory = peak_memory(stderr)
                    memory_method = 'resource.getrusage(RUSAGE_CHILDREN), fresh one-child process'
                    memory_reason = stderr.strip()[:300] or 'getrusage metric absent.'
                else:
                    memory_reason = 'getrusage probe differs from the direct native probe.'
            except (OSError, ValueError, TimeoutError, subprocess.TimeoutExpired) as error:
                memory_reason = str(error)
        entry['native']['peakMemoryBytes'] = memory
        entry['native']['peakMemoryMethod'] = memory_method if memory is not None else None
        if memory is None:
            report['skipped'].append({'file': entry['file'], 'reason': 'Peak memory unavailable: ' + memory_reason})
        if 'error' in native or 'error' in wasm:
            entry['status'] = 'unsupported'
            entry['errors'] = {'native': native.get('error'), 'wasm': wasm.get('error')}
            report['unsupported'].append(entry['file'])
        else:
            fields = [field for field in sorted(native.keys() | wasm.keys()) if native.get(field) != wasm.get(field)]
            entry['differentFields'] = fields
            entry['status'] = 'different' if fields else 'matched'
            if fields:
                report['differences'].append({'file': entry['file'], 'fields': fields})
    report['matched'] = sum(entry.get('status') == 'matched' for entry in report['files'])
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 1 if report['differences'] or report['unsupported'] else 0


if __name__ == '__main__':
    sys.exit(main())
