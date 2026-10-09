import {test} from 'node:test';
import assert from 'node:assert/strict';
import {PanelZoom} from '../panel-zoom.js';
import {PageView} from '../page-view.js';

function setup(width = 720) {
  const frames = [];
  const view = {
    entries: [{width: 794, page: {style: {}}}],
    canvas: {clientWidth: width},
    zoom: 100,
    fit: false,
    onZoom() {},
    sizePage: PageView.prototype.sizePage,
    setZoom: PageView.prototype.setZoom
  };
  const panel = new PanelZoom(view, callback => frames.push(callback));
  return {view, panel, settle: () => frames.splice(0).forEach(callback => callback())};
}

test('panel fits only overflow and restores zoom unless the person changed it', () => {
  const {view, panel, settle} = setup();
  panel.update(true);
  settle();
  assert.equal(view.fit, true);
  panel.update(false);
  assert.equal(view.zoom, 100);
  panel.update(true);
  settle();
  panel.changed();
  view.setZoom(60);
  panel.update(false);
  assert.equal(view.zoom, 60);
  view.canvas.clientWidth = 1200;
  panel.update(true);
  settle();
  assert.equal(view.zoom, 60);
});

test('panel fits the settled canvas width including padding and proof margins', () => {
  const {view, panel, settle} = setup(1324);
  panel.update(true);
  assert.equal(view.zoom, 100);
  view.canvas.clientWidth = 917;
  settle();
  assert.equal(view.fit, true);
  assert.ok(view.entries[0].width * view.zoom / 100 + 216 <= 917);
});

test('fit width accounts for the widest page, including landscape pages', () => {
  const {view} = setup(917);
  view.entries.push({width: 1123, page: {style: {}}});
  view.setZoom('fit');
  assert.ok(Math.abs(view.zoom - (917 - 216) / 1123 * 100) < 0.001);
});

test('closing the panel or changing zoom cancels pending automatic fit', () => {
  for (const cancel of ['close', 'change']) {
    const {view, panel, settle} = setup();
    panel.update(true);
    if (cancel === 'close') {
      panel.update(false);
    } else {
      panel.changed();
      view.setZoom(125);
    }
    settle();
    assert.equal(view.fit, false);
    assert.equal(view.zoom, cancel === 'close' ? 100 : 125);
  }
});
