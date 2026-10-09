"""Bounded, test-only rhwp reopen/render check; never used to save documents."""
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PROBE_SOURCE = r'''
import fs from 'node:fs';
import init, { HwpDocument } from './node_modules/@rhwp/core/rhwp.js';
globalThis.measureTextWidth = (font, text) => text.length * 10;
await init({ module_or_path: fs.readFileSync('./node_modules/@rhwp/core/rhwp_bg.wasm') });
const document = new HwpDocument(new Uint8Array(fs.readFileSync(0)));
try {
    const pageCount = document.pageCount();
    if (pageCount < 1) {
        throw new Error('rhwp returned no pages.');
    }
    const svg = document.renderPageSvg(0);
    if (!svg.includes('<svg')) {
        throw new Error('rhwp did not return a page SVG.');
    }
    console.log(JSON.stringify({ pageCount, renderedPage: 0, text: document.getTextFileUnicode() }));
} finally {
    document.free();
}
'''


def probe_rhwp(payload, timeout=10):
    node = shutil.which('node')
    if node is None:
        return {'status': 'skipped', 'reason': 'Node.js is unavailable.'}
    package = ROOT / 'node_modules/@rhwp/core'
    required = ['package.json', 'rhwp.js', 'rhwp_bg.wasm']
    if not all((package / name).is_file() for name in required):
        return {'status': 'skipped', 'reason': '@rhwp/core or its WASM file is unavailable.'}
    try:
        version = json.loads((package / 'package.json').read_text())['version']
        result = subprocess.run(
            [node, '--input-type=module', '-e', PROBE_SOURCE],
            input=payload,
            capture_output=True,
            cwd=ROOT,
            timeout=timeout,
            check=False,
        )
        if result.returncode != 0:
            return {
                'status': 'failed',
                'version': version,
                'reason': result.stderr.decode('utf-8', errors='replace')[-4000:],
            }
        observed = json.loads(result.stdout)
        # getTextFileUnicode() returns a JSON string in rhwp 0.8.7.
        # The TEXT variant escapes characters outside CP949 as numeric references.
        text = json.loads(observed['text'])
        return {'status': 'passed', 'version': version, **observed, 'text': text}
    except FileNotFoundError:
        return {'status': 'skipped', 'reason': 'Node.js became unavailable.'}
    except subprocess.TimeoutExpired:
        return {'status': 'failed', 'reason': f'rhwp exceeded the {timeout}-second timeout.'}
    except (OSError, ValueError, KeyError) as error:
        return {'status': 'failed', 'reason': str(error)}
