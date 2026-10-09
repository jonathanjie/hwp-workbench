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

test('requested Windows font list uses exactly the renderer substitution source', async () => {
  const {FONT_SUBSTITUTIONS, requestedFontSubstitutions, macFontFamily} = await import('../fonts.js');
  const list = requestedFontSubstitutions(Object.keys(FONT_SUBSTITUTIONS), () => false);
  assert.deepEqual(list, Object.entries(FONT_SUBSTITUTIONS).map(([from, to]) => ({from, to})));
  assert.equal(macFontFamily('궁서'), 'AppleMyungjo');
  assert.deepEqual(requestedFontSubstitutions(['굴림', '바탕'], name => name === '굴림'), [{from: '바탕', to: 'AppleMyungjo'}]);
});
