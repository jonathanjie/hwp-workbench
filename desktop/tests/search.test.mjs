import {test} from 'node:test';
import assert from 'node:assert/strict';
import {EditModel} from '../edit-model.js';
import {findMatches, replaceMatches} from '../search.js';

function model(lengthChangingText = true) {
  return new EditModel({format: 'hwp', sha256: 'hash', capabilities: {lengthChangingText}, nodes: [
    {id: 'a', text: 'Cat cat', editable: true},
    {id: 'b', text: 'cat', editable: false},
    {id: 'c', text: 'cat', editable: true}
  ]});
}

test('find uses literal UTF-16 ranges, includes locked text and respects case', () => {
  const edits = model();
  assert.equal(findMatches(edits, 'cat').length, 4);
  assert.equal(findMatches(edits, 'cat', true).length, 3);
  assert.equal(findMatches(edits, '[cat]').length, 0);
  assert.equal(findMatches(edits, '').length, 0);
});

test('replace all groups edits by node, skips locks and undoes in one step', () => {
  const edits = model();
  const result = replaceMatches(edits, findMatches(edits, 'cat'), 'kitten');
  assert.equal(result.replaced, 3);
  assert.deepEqual(result.skipped.map(item => item.reason), ['locked']);
  assert.equal(edits.current(edits.nodes.get('a')), 'kitten kitten');
  assert.equal(edits.patch().operations.length, 2);
  assert.equal(edits.patch().operations[0].expectedText, 'Cat cat');
  edits.undo();
  assert.equal(edits.patch().operations.length, 0);
  edits.redo();
  assert.equal(edits.patch().operations.length, 2);
});

test('replace honors capability, rejects controls and stale ranges without losing prior edits', () => {
  const edits = model(false);
  assert.equal(replaceMatches(edits, findMatches(edits, 'cat'), 'kitten').replaced, 0);
  assert.equal(replaceMatches(edits, findMatches(edits, 'cat'), 'dog').replaced, 3);
  const snapshot = edits.patch();
  assert.equal(replaceMatches(edits, findMatches(edits, 'dog'), '\n').replaced, 0);
  assert.deepEqual(edits.patch(), snapshot);
  const stale = [{node: edits.nodes.get('c'), text: 'cat', start: 0, end: 3}];
  assert.equal(replaceMatches(edits, stale, 'foo').skipped[0].reason, 'sourceChanged');
});
