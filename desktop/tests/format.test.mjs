import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readRhwpFormat, formatRows} from '../format-card.js';

test('read-only format uses native properties and marks unknown fields', () => {
  const source = {getCharPropertiesAt: () => JSON.stringify({fontFamily: '굴림', fontSize: 1200, bold: false}),
    getParaPropertiesAt: () => '{}', getPageDef: () => '{}', getStyleAt: () => '{}'};
  const result = readRhwpFormat(source, {anchor: {section: 0, paragraph: 1, charOffset: 0, inCell: false}});
  assert.equal(result.character.size, 12);
  assert.equal(result.character.font, '굴림');
  const rows = formatRows('charFormat', result);
  assert.equal(rows.find(row => row.key === 'widthRatio').value, '—');
  assert.equal(rows.find(row => row.key === 'bold').value, false);
  assert.deepEqual(readRhwpFormat(source, {anchor: {inCell: true}}).character, {});
});
