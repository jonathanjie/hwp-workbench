import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {rustRpc} from '../../scripts/rust-cli.mjs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {previewNodes} from '../preview.js';
import {mapTextRuns} from '../hit-map.js';

const root = fileURLToPath(new URL('../../', import.meta.url));

function rpc(method, args) {
  return rustRpc(root, method, args);
}

test('native staged bytes render edited text and retain body and table hit identities without a GUI', async t => {
  let runtime;
  let wasm;
  try {
    runtime = await import('../../node_modules/@rhwp/core/rhwp.js');
    wasm = readFileSync(path.join(root, 'node_modules/@rhwp/core/rhwp_bg.wasm'));
  } catch {
    t.skip('rhwp package unavailable; native page-layout check not run');
    return;
  }
  globalThis.measureTextWidth = (_font, text) => [...text].length * 10;
  await runtime.default({module_or_path: wasm});
  const source = 'fixtures/generated/review-form.hwpx';
  const inspected = rpc('inspect', {path: source});
  const node = inspected.nodes.find(node => node.editable && node.role === 'body');
  const text = node.text + ' 한글😀';
  const cell = inspected.nodes.find(node => node.editable && node.role === 'table-cell');
  const cellText = cell.text + ' C3 table';
  const patch = {schemaVersion: 1, baseSha256: inspected.sha256, operations: [
    {op: 'set_text', nodeId: node.id, expectedText: node.text, text},
    {op: 'set_text', nodeId: cell.id, expectedText: cell.text, text: cellText}
  ]};
  const preview = rpc('preview', {path: source, patch});
  const document = new runtime.HwpDocument(new Uint8Array(Buffer.from(preview.bytesBase64, 'base64')));
  try {
    const runs = Array.from({length: document.pageCount()}, (_, page) => JSON.parse(document.getPageTextLayout(page)).runs).flat();
    assert.ok(runs.map(run => run.text).join('').includes('한글😀'));
    const mapped = mapTextRuns(runs, previewNodes(inspected.nodes, {[node.id]: text, [cell.id]: cellText}));
    assert.ok(mapped.some(item => item.node?.id === node.id && item.run.text.includes('한글')));
    assert.ok(mapped.some(item => item.node?.id === cell.id && item.run.text.includes('C3')));
  } finally {
    document.free();
  }
});
