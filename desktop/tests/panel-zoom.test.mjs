import {test} from 'node:test';
import assert from 'node:assert/strict';
import {PanelZoom} from '../panel-zoom.js';

test('panel fits only overflow and restores zoom unless the person changed it', () => {
  let value = 100;
  let fit = false;
  const view = {entries: [{width: 794}], canvas: {clientWidth: 720},
    get zoom() { return value; }, get fit() { return fit; },
    setZoom(next) { fit = next === 'fit'; value = fit ? 77 : next; }};
  const panel = new PanelZoom(view);
  panel.update(true);
  assert.equal(fit, true);
  panel.update(false);
  assert.equal(value, 100);
  panel.update(true);
  panel.changed();
  view.setZoom(60);
  panel.update(false);
  assert.equal(value, 60);
  view.canvas.clientWidth = 1200;
  panel.update(true);
  assert.equal(value, 60);
});

test('panel opening accounts for canvas padding and page margin space', () => {
  let fitted = false;
  const view = {entries: [{width: 794}], canvas: {clientWidth: 917}, zoom: 100, fit: false,
    setZoom: value => { fitted = value === 'fit'; }};
  new PanelZoom(view).update(true);
  assert.equal(fitted, true);
});
