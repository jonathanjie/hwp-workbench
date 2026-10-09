import {test} from 'node:test';
import assert from 'node:assert/strict';
import {wordDiffSegments} from '../changes.js';

test('review keeps unchanged words between separate edits and preserves both texts', () => {
  const segments = wordDiffSegments('one two three four five', 'one TWO three FOUR five');
  assert.equal(segments.filter(item => item.type !== 'ins').map(item => item.text).join(''), 'one two three four five');
  assert.equal(segments.filter(item => item.type !== 'del').map(item => item.text).join(''), 'one TWO three FOUR five');
  assert.ok(segments.some(item => item.type === 'same' && item.text.includes('three')));
});
