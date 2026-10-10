import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mapTextRuns, paragraphBounds} from '../hit-map.js';

const run = (text, paraIdx, charStart = 0, secIdx = 0) => ({text, paraIdx, charStart, secIdx, x: 10, y: 20, w: 40, h: 12});
const node = (id, text, part = 'Contents/section0.xml', anchor) => ({id, text, part, editable: true, anchor});

test('native marker glyphs map back to editable body and consent-cell nodes', () => {
  for (const inCell of [false, true]) {
    const marked = {...node('consent', '□\ufdd0동의함', undefined, {section: 0, paragraph: 0, charOffset: inCell ? null : 0, inCell}),
      inlineMarkers: {'\ufdd0': '\u2007'}, paragraph: 1};
    const glyphRun = {...run('□\u2007동의함', 0), inCell};
    assert.equal(mapTextRuns([glyphRun], [marked])[0].node, marked);
  }
});

test('paired empty markers do not advance native hit ranges', () => {
  const marked = {...node('paired', 'A\ufdd0B', undefined, {section: 0, paragraph: 0, charOffset: 0, inCell: false}), inlineMarkers: {'\ufdd0': ''}};
  assert.equal(mapTextRuns([run('AB', 0)], [marked])[0].node, marked);
});

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

// A nested run: a table cell or text-box paragraph inside top-level paragraph `top`.
const nested = (text, top, charStart = 0, cellIndex = 0, cellParaIndex = 0) => ({...run(text, cellParaIndex, charStart), parentParaIdx: top,
  cellIdx: cellIndex, cellParaIdx: cellParaIndex, cellPath: [{controlIndex: 0, cellIndex, cellParaIndex}]});
const cellNode = (id, text, top, paragraph) => ({...node(id, text, undefined, {section: 0, paragraph: top, charOffset: null, inCell: true}), paragraph});

test('a floating text box drawn out of document order does not take a later table cell', () => {
  // XML order: box (top 0), cells (tops 1-3). Layout draws the box after two cells.
  const nodes = [cellNode('box', 'Note 1', 0, 10), cellNode('name', 'Name', 1, 11), cellNode('address', 'Address', 2, 12), cellNode('note', 'Note 1 applies', 3, 13)];
  const runs = [nested('Name', 1), nested('Address', 2), nested('Note 1', 0), nested('Note 1 applies', 3)];
  assert.deepEqual(mapTextRuns(runs, nodes).map(item => item.node?.id), ['name', 'address', 'box', 'note']);
});

test('repeated form blocks map each copy to its own table', () => {
  const nodes = [cellNode('a-box', '□ agree', 1, 10), cellNode('a-name', 'Name', 1, 11), cellNode('b-box', '□ agree', 2, 12), cellNode('b-name', 'Name', 2, 13)];
  const runs = [nested('□ agree', 1), nested('Name', 1, 0, 1), nested('□ agree', 2), nested('Name', 2, 0, 1)];
  assert.deepEqual(mapTextRuns(runs, nodes).map(item => item.node?.id), ['a-box', 'a-name', 'b-box', 'b-name']);
  // A page shows only some runs; the mapping still uses the whole document order.
  assert.equal(mapTextRuns([runs[3]], nodes, runs)[0].node.id, 'b-name');
});

test('a run that crosses two native text nodes opens the node where it starts', () => {
  const nodes = [cellNode('site', 'www.example.kr', 4, 20), cellNode('rest', ') in Seoul', 4, 20)];
  const runs = [nested('www.example.kr)', 4), nested(' in Seoul', 4, 15)];
  assert.deepEqual(mapTextRuns(runs, nodes).map(item => item.node?.id), ['site', 'rest']);
});

test('text-box runs split by style follow native order across node boundaries', () => {
  const nodes = [cellNode('number', '1', 3, 12), cellNode('space', ' ', 3, 12), cellNode('title', 'Title text', 3, 12), cellNode('next', 'Second line', 3, 13)];
  const runs = [nested('1', 3), nested('Title tex', 3, 2), nested('t', 3, 11), nested('Second line', 3, 0, 0, 1)];
  assert.deepEqual(mapTextRuns(runs, nodes).map(item => item.node?.id), ['number', 'title', 'title', 'next']);
});

test('two text boxes in one paragraph map in control order, not drawing order', () => {
  const nodes = [cellNode('first-box', 'Signature box', 3, 12), cellNode('second-box', 'Seal', 3, 13)];
  const box = (text, controlIndex) => ({...nested(text, 3), cellPath: [{controlIndex, cellIndex: 0, cellParaIndex: 0}]});
  const runs = [box('Seal', 1), box('Signature box', 0)];
  assert.deepEqual(mapTextRuns(runs, nodes).map(item => item.node?.id), ['second-box', 'first-box']);
});

test('whitespace-only runs do not move the native text cursor past later text', () => {
  const nodes = [cellNode('name', 'Name', 5, 30), cellNode('seal', ' (seal)', 5, 31)];
  const runs = [nested(' ', 5), nested('Name', 5, 0, 1), nested('(seal)', 5, 0, 2)];
  assert.deepEqual(mapTextRuns(runs, nodes).map(item => item.node?.id), ['seal', 'name', 'seal']);
});

test('a cleared anchored paragraph keeps its identity for proof marks and later editing', () => {
  const node = {id: 'empty', text: '', anchor: {section: 0, paragraph: 1, charOffset: 0, inCell: false}};
  const run = {text: '', secIdx: 0, paraIdx: 1, charStart: 0, x: 20, y: 30, w: 0, h: 12, charX: [0]};
  assert.equal(mapTextRuns([run], [node])[0].node?.id, 'empty');
});
