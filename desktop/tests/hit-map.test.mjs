import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mapTextRuns, paragraphBounds} from '../hit-map.js';

const run = (text, paraIdx, charStart = 0, secIdx = 0) => ({text, paraIdx, charStart, secIdx, x: 10, y: 20, w: 40, h: 12});
const node = (id, text, part = 'Contents/section0.xml', anchor) => ({id, text, part, editable: true, anchor});

test('anchor maps a partial run to its native text span and excludes other sections', () => {
  const nodes = [node('wrong', 'repeat', 'Contents/section1.xml'), node('right', 'repeat tail', undefined, {section: 0, paragraph: 2, charOffset: 2, inCell: false})];
  assert.equal(mapTextRuns([run('repeat', 2, 2)], nodes)[0].node.id, 'right');
});

test('fallback maps equal paragraphs by their document position, including later-page runs', () => {
  const nodes = [node('first', 'repeat'), node('second', 'repeat')];
  const all = [run('repeat', 0), run('repeat', 1)];
  assert.deepEqual(mapTextRuns(all, nodes).map(item => item.node.id), ['first', 'second']);
  assert.equal(mapTextRuns([all[1]], nodes, all)[0].node.id, 'second');
});

test('split runs stay on one containing node; unmatched runs stay locked', () => {
  const runs = [run('Hello', 0), run(' world', 0, 5), run('unknown', 1)];
  assert.deepEqual(mapTextRuns(runs, [node('whole', 'Hello world')]).map(item => item.node?.id), ['whole', 'whole', undefined]);
});

test('paragraph bounds cover all runs and keep separate table cells separate', () => {
  const runs = [{...run('a', 0), cellAddr: [0, 0]}, {...run('b', 0), x: 80, y: 40, cellAddr: [0, 0]}, {...run('c', 0), x: 200, cellAddr: [0, 1]}];
  const groups = paragraphBounds(runs);
  assert.equal(groups.length, 2);
  assert.deepEqual(groups[0].bounds, {x: 10, y: 20, w: 110, h: 32});
});

test('table-local paragraph indexes do not consume earlier body nodes or merge separate tables', () => {
  const runs = [run('Body', 0), run('Next body', 1),
    {...run('Cell', 0), parentParaIdx: 2, cellIdx: 0, cellParaIdx: 0, cellPath: [0]},
    {...run('Other cell', 0), parentParaIdx: 4, cellIdx: 0, cellParaIdx: 0, cellPath: [0]}];
  const nodes = [node('body', 'Body'), node('next', 'Next body'), node('cell', 'Cell'), node('other', 'Other cell')];
  assert.deepEqual(mapTextRuns(runs, nodes).map(item => item.node?.id), ['body', 'next', 'cell', 'other']);
  assert.equal(paragraphBounds(runs).length, 4);
});

test('a cleared anchored paragraph keeps its identity for proof marks and later editing', () => {
  const node = {id: 'empty', text: '', anchor: {section: 0, paragraph: 1, charOffset: 0, inCell: false}};
  const run = {text: '', secIdx: 0, paraIdx: 1, charStart: 0, x: 20, y: 30, w: 0, h: 12, charX: [0]};
  assert.equal(mapTextRuns([run], [node])[0].node?.id, 'empty');
});
