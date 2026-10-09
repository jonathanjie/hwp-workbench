import {test} from 'node:test';
import assert from 'node:assert/strict';
import {previewNodes, changedRanges} from '../preview.js';
import {mapTextRuns} from '../hit-map.js';

test('preview uses current text and shifts later native anchors in the same paragraph', () => {
  const nodes = [
    {id: 'a', text: 'old', paragraph: 1, anchor: {section: 0, paragraph: 0, charOffset: 0, inCell: false}},
    {id: 'b', text: ' tail', paragraph: 1, anchor: {section: 0, paragraph: 0, charOffset: 3, inCell: false}}
  ];
  const current = previewNodes(nodes, {a: 'new words'});
  assert.equal(current[1].anchor.charOffset, 9);
  const run = {secIdx: 0, paraIdx: 0, inCell: false, charStart: 9, text: ' tail', x: 0, y: 0, w: 10, h: 10};
  assert.equal(mapTextRuns([run], current)[0].node.id, 'b');
  assert.equal(nodes[1].anchor.charOffset, 3);
});

test('range marks keep separate word edits apart and retain a deletion point', () => {
  assert.deepEqual(changedRanges('one two three four', 'one TWO three FOUR'), [
    {start: 4, end: 7}, {start: 14, end: 18}
  ]);
  assert.deepEqual(changedRanges('one two', 'one '), [{start: 4, end: 4}]);
});

test('a background preview failure restores the previous pages and frees the failed view', async () => {
  const {commitPagePreview} = await import('../preview.js');
  const host = children => ({children, replaceChildren(...next) { this.children = next; }});
  const pages = host(['previous-page']);
  const thumbnails = host(['previous-thumb']);
  let active;
  let failure;
  let released = 0;
  const previous = {pages, thumbnails, suspend() {}, activate() {}, setZoom() {}, clear() { released += 1; }};
  let failedFreed = 0;
  const next = {pages: host(['next-page']), thumbnails: host(['next-thumb']), activate() {}, setZoom() {}, clear() { failedFreed += 1; }};
  commitPagePreview(previous, next, {pages, thumbnails, onMount: view => { active = view; }, onRollback: error => { failure = error; }});
  assert.equal(active, next);
  assert.deepEqual(pages.children, ['next-page']);
  next.onFailure(Error('render failed'));
  assert.equal(active, previous);
  assert.deepEqual(pages.children, ['previous-page']);
  assert.deepEqual(thumbnails.children, ['previous-thumb']);
  assert.equal(failure.message, 'render failed');
  assert.equal(released, 0);
  assert.equal(failedFreed, 1);
});

test('native anchors and hit ranges count Unicode scalars after an emoji edit', () => {
  const nodes = [
    {id: 'a', text: 'old', paragraph: 1, anchor: {section: 0, paragraph: 0, charOffset: 0, inCell: false}},
    {id: 'b', text: 'BCBC', paragraph: 1, anchor: {section: 0, paragraph: 0, charOffset: 3, inCell: false}}
  ];
  const current = previewNodes(nodes, {a: 'A😀'});
  assert.equal(current[1].anchor.charOffset, 2);
  const run = {secIdx: 0, paraIdx: 0, inCell: false, charStart: 4, text: 'BC', x: 0, y: 0, w: 10, h: 10};
  assert.equal(mapTextRuns([run], current)[0].node.id, 'b');
});

test('preview mount keeps the current zoom and refits after asynchronous rendering', async () => {
  const {commitPagePreview} = await import('../preview.js');
  const host = children => ({children, replaceChildren(...items) { this.children = items; }});
  for (const fit of [false, true]) {
    const pages = host(['old']);
    const thumbnails = host(['old-thumb']);
    const previous = {pages, thumbnails, zoom: 75, fit, suspend() {}, activate() {}, clear() {}, setZoom() {}};
    const next = {
      pages: host(['new']), thumbnails: host(['new-thumb']), zoom: 100, fit: false,
      activate() {}, clear() {},
      setZoom(value) { this.requestedZoom = value; }
    };
    commitPagePreview(previous, next, {pages, thumbnails, onMount() {}, onRollback() {}});
    assert.equal(next.requestedZoom, fit ? 'fit' : 75);
  }
});
