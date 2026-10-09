import {test} from 'node:test';
import assert from 'node:assert/strict';
import {EditModel, textError} from '../edit-model.js';

const node = {id: 'body#1', text: 'Original', editable: true};
const document = {sha256: 'source-hash', format: 'hwpx', capabilities: {lengthChangingText: true}, nodes: [node]};

test('stage, revert and undo/redo retain expected original text in hash-bound patches', () => {
  const model = new EditModel(document);
  assert.equal(model.stage(node, 'Changed'), true);
  assert.equal(model.patch().baseSha256, document.sha256);
  assert.deepEqual(model.patch().operations, [{op: 'set_text', nodeId: node.id, expectedText: 'Original', text: 'Changed'}]);
  model.undo();
  assert.equal(model.patch().operations.length, 0);
  model.redo();
  assert.equal(model.current(node), 'Changed');
  model.revert(node.id);
  assert.equal(model.patch().operations.length, 0);
  model.undo();
  assert.equal(model.current(node), 'Changed');
});

test('draft recovery refuses stale expected text, controls and read-only edits', () => {
  const locked = {id: 'locked', text: 'Locked', editable: false};
  const model = new EditModel({...document, nodes: [node, locked]});
  model.restore({schemaVersion: 1, baseSha256: document.sha256, operations: [
    {op: 'set_text', nodeId: node.id, expectedText: 'stale', text: 'Change'},
    {op: 'set_text', nodeId: locked.id, expectedText: locked.text, text: 'Change'},
    {op: 'set_text', nodeId: node.id, expectedText: node.text, text: 'bad\ntext'}
  ]});
  assert.equal(model.patch().operations.length, 0);
  assert.equal(textError('😀', 'ab', false), null);
  assert.equal(textError('😀', 'a', false), 'lengthError');
  assert.equal(textError('\ud800', '', true), 'controls');
});

test('agent authors survive undo/redo and a human edit replaces the origin', () => {
  const model = new EditModel(document);
  assert.equal(model.stageBatch([{node, text: 'Agent changed', author: 'Claude'}]), true);
  assert.equal(model.origins[node.id], 'Claude');
  model.undo();
  assert.equal(model.origins[node.id], undefined);
  model.redo();
  assert.equal(model.origins[node.id], 'Claude');
  model.stage(node, 'Human changed');
  assert.equal(model.origins[node.id], undefined);
  model.undo();
  assert.equal(model.origins[node.id], 'Claude');
});
