import {test} from 'node:test';
import assert from 'node:assert/strict';
import {matchBounds} from '../search-layout.js';

test('find marks use native character positions across split runs and Unicode offsets', () => {
  const node = {id: 'n', text: 'A😀BC', anchor: {charOffset: 4}};
  const run = {text: '😀BC', charStart: 5, x: 20, y: 30, h: 12, charX: [0, 15, 23, 32]};
  assert.deepEqual(matchBounds(run, node, {node, text: node.text, start: 1, end: 4}), {x: 20, y: 30, w: 23, h: 12});
  assert.equal(matchBounds(run, node, {node, text: 'changed', start: 1, end: 4}), null);
});

test('repeated text after a supplementary character keeps its native run position', () => {
  const node = {id: 'n', text: 'A😀BCBC', anchor: {charOffset: 0}};
  const run = {text: 'BC', charStart: 4, x: 10, y: 20, h: 12, charX: [0, 5, 10]};
  assert.deepEqual(matchBounds(run, node, {node, text: node.text, start: 5, end: 7}), {x: 10, y: 20, w: 10, h: 12});
});
