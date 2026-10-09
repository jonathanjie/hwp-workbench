import {test} from 'node:test';
import assert from 'node:assert/strict';
import {substituteFontFamily} from '../fonts.js';

test('missing document fonts use one quoted family and a generic fallback for canvas and SVG', () => {
  const changes = [];
  assert.equal(substituteFontFamily('"없는 고딕", sans-serif', () => false, (from, to) => changes.push({from, to})), '"Apple SD Gothic Neo", sans-serif');
  assert.equal(substituteFontFamily('바탕', () => false), '"AppleMyungjo", serif');
  assert.equal(substituteFontFamily('Arial', family => family === 'Arial'), '"Arial", sans-serif');
  assert.deepEqual(changes, [{from: '없는 고딕', to: 'Apple SD Gothic Neo'}]);
});
