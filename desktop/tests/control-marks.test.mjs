import {test} from 'node:test';
import assert from 'node:assert/strict';
import {controlMarks} from '../control-marks.js';
test('control overlay covers bounds, locked runs and paragraph ends without mutation', () => {
  const run = {x: 5, y: 10, w: 40, h: 12};
  const controls = [{type: 'table', x: 0, y: 0, w: 100, h: 100, cells: [{x: 0, y: 0, w: 50, h: 50}]},
    {type: 'field', x: 10, y: 10, w: 20, h: 10}];
  const before = JSON.stringify(controls);
  const marks = controlMarks(controls, [{run, node: {editable: false}}], [{runs: [run]}]);
  assert.deepEqual(marks.map(item => item.type), ['table', 'cell', 'field', 'locked', 'paragraphEnd']);
  assert.equal(JSON.stringify(controls), before);
});

test('inline field marks use native field membership at mapped text offsets', async () => {
  const {inlineFieldMarks} = await import('../control-marks.js');
  const source = {getFieldInfoAt: (section, paragraph, offset) => {
    assert.equal(section, 0);
    assert.equal(paragraph, 2);
    return JSON.stringify({inField: offset === 3});
  }};
  const mapped = [{node: {anchor: {inCell: false}}, run: {secIdx: 0, paraIdx: 2, charStart: 3, text: '필드', x: 1, y: 2, w: 20, h: 10}}];
  assert.equal(inlineFieldMarks(source, mapped)[0].type, 'field');
  assert.deepEqual(inlineFieldMarks(source, [{...mapped[0], node: {anchor: {inCell: true}}}]), []);
});
