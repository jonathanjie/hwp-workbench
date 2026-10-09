"""Byte-only, bounded adapter to the optional independent rhwp parser."""
import json
import os
from pathlib import Path
import selectors
import shutil
import struct
import subprocess
import time

from .errors import MAX_FILE, WorkbenchError, require

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / 'node_modules/@rhwp/core'
BRIDGE = ROOT / 'engine/bridge/rhwp-bridge.mjs'
TIMEOUT = 30
MAX_OUTPUT = 64 * 1024 * 1024
CHUNK_SIZE = 64 * 1024


def _run_bridge(command, framed_input):
    """Drain both output pipes while writing stdin; kill before bounds are exceeded."""
    output = bytearray()
    received = 0
    deadline = time.monotonic() + TIMEOUT
    try:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, shell=False)
    except FileNotFoundError:
        return {'available': False, 'reason': 'Node.js became unavailable.'}
    except OSError as error:
        raise WorkbenchError('INDEPENDENT_FAILED', str(error)) from error
    with process, selectors.DefaultSelector() as selector:
        streams = [process.stdin, process.stdout, process.stderr]
        for stream in streams:
            os.set_blocking(stream.fileno(), False)
        selector.register(process.stdin, selectors.EVENT_WRITE)
        selector.register(process.stdout, selectors.EVENT_READ)
        selector.register(process.stderr, selectors.EVENT_READ)
        sent = 0
        try:
            while selector.get_map():
                remaining = deadline - time.monotonic()
                require(remaining > 0, 'INDEPENDENT_TIMEOUT', 'rhwp exceeded the 30-second timeout.')
                for key, _ in selector.select(remaining):
                    stream = key.fileobj
                    if stream is process.stdin:
                        try:
                            sent += os.write(stream.fileno(), framed_input[sent:sent + CHUNK_SIZE])
                        except BrokenPipeError:
                            sent = len(framed_input)
                        if sent == len(framed_input):
                            selector.unregister(stream)
                            stream.close()
                        continue
                    chunk = os.read(stream.fileno(), CHUNK_SIZE)
                    if not chunk:
                        selector.unregister(stream)
                        continue
                    received += len(chunk)
                    require(received <= MAX_OUTPUT, 'INDEPENDENT_LIMIT', 'rhwp output exceeds 64 MiB.')
                    if stream is process.stdout:
                        output.extend(chunk)
            remaining = deadline - time.monotonic()
            require(remaining > 0, 'INDEPENDENT_TIMEOUT', 'rhwp exceeded the 30-second timeout.')
            process.wait(timeout=remaining)
        except subprocess.TimeoutExpired as error:
            process.kill()
            raise WorkbenchError('INDEPENDENT_TIMEOUT', 'rhwp exceeded the 30-second timeout.') from error
        except BaseException:
            process.kill()
            raise
    try:
        result = json.loads(output)
        require(isinstance(result, dict), 'INDEPENDENT_PROTOCOL', 'rhwp returned a non-object response.')
        if 'error' in result:
            error = result['error']
            require(isinstance(error, dict), 'INDEPENDENT_PROTOCOL', 'Invalid rhwp error response.')
            raise WorkbenchError(str(error.get('code', 'RHWP_FAILED')), str(error.get('message', 'rhwp failed.')))
        require(process.returncode == 0, 'INDEPENDENT_FAILED', 'rhwp exited without a valid result.')
        return result
    except (ValueError, UnicodeError) as error:
        raise WorkbenchError('INDEPENDENT_PROTOCOL', 'rhwp returned invalid JSON.') from error


def invoke(payload, op='probe', *, page=None, format=None):
    require(isinstance(payload, bytes) and 0 < len(payload) <= MAX_FILE,
            'LIMIT', 'Independent parser input must be between 1 byte and 64 MiB.')
    node_setting = os.environ.get('HWP_WORKBENCH_NODE')
    node = shutil.which(node_setting or 'node')
    if node is None:
        return {'available': False, 'reason': 'Node.js is unavailable (HWP_WORKBENCH_NODE or PATH).'}
    if not all((PACKAGE / name).is_file() for name in ('package.json', 'rhwp.js', 'rhwp_bg.wasm')):
        return {'available': False, 'reason': '@rhwp/core or its WASM file is unavailable.'}
    request = {'op': op}
    if page is not None:
        request['page'] = page
    if format is not None:
        request['format'] = format
    encoded = json.dumps(request, separators=(',', ':')).encode('utf-8')
    framed = struct.pack('<I', len(encoded)) + encoded + payload
    return _run_bridge([node, str(BRIDGE)], framed)


def probe(payload):
    return invoke(payload, 'probe')


def check_output(payload, edited_texts):
    """Report independent evidence without overriding native save verification."""
    try:
        observed = probe(payload)
        if observed.get('available') is False:
            return observed
        texts = [paragraph['text'] for section in observed['sections']
                 for paragraph in section['paragraphs']]
        missing = [node_id for node_id, text in edited_texts.items()
                   if not any(text in paragraph for paragraph in texts)]
        return {'engine': f"rhwp {observed['version']}", 'opened': True,
                'pageCount': observed['pageCount'], 'editedTextsFound': not missing,
                'missing': missing}
    except WorkbenchError as error:
        return {'engine': 'rhwp 0.8.7', 'opened': False, 'error': error.as_dict()}
