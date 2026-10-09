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

test('unknown, disabled and unsupported format states remain distinct in both languages', async () => {
  const {formatValue} = await import('../format-card.js');
  const {labelFor} = await import('../i18n.js');
  for (const locale of ['en', 'ko']) {
    const translate = key => labelFor(locale, key);
    const rows = formatRows('charFormat', {character: {bold: false}});
    for (const key of ['italic', 'underline']) {
      const row = rows.find(item => item.key === key);
      assert.equal(formatValue('charFormat', key, row.value, translate), '—');
    }
    assert.equal(formatValue('charFormat', 'bold', false, translate), locale === 'ko' ? '꺼짐' : 'Off');
    assert.equal(formatValue('charFormat', 'bold', true, translate), locale === 'ko' ? '켜짐' : 'On');
    assert.equal(labelFor(locale, 'cellSlip'), locale === 'ko' ? '셀 블록' : 'Cell block');
  }
});

test('format card centres without a slip and chooses an adjacent side within the viewport', async () => {
  const {formatCardPosition} = await import('../format-card.js');
  const card = {width: 360, height: 330};
  const viewport = {width: 1440, height: 900};
  assert.deepEqual(formatCardPosition(card, viewport), {left: 540, top: 285});
  assert.deepEqual(formatCardPosition(card, viewport, {left: 400, right: 870, top: 300, bottom: 540}), {left: 882, top: 300});
  assert.deepEqual(formatCardPosition(card, viewport, {left: 750, right: 1220, top: 790, bottom: 880}), {left: 378, top: 558});
});

test('format card stays within a narrow viewport and below the window band', async () => {
  const {formatCardPosition} = await import('../format-card.js');
  const position = formatCardPosition({width: 360, height: 330}, {width: 800, height: 720},
    {left: 170, right: 640, top: 70, bottom: 250});
  assert.deepEqual(position, {left: 170, top: 262});
});
