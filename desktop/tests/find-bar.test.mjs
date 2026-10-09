import {test} from 'node:test';
import assert from 'node:assert/strict';
import {FindBar} from '../find-bar.js';
import {EditModel} from '../edit-model.js';
import {findMatches} from '../search.js';

test('replace refuses to change the model while the proof slip holds an unstaged draft', () => {
  const node = {id: 'n', text: 'draft target', editable: true};
  const model = new EditModel({sha256: 'hash', capabilities: {lengthChangingText: true}, nodes: [node]});
  let guardCalled = false;
  const control = {replaceChildren() {}, showPopover() {}};
  const bar = {
    getModel: () => model,
    matches: findMatches(model, 'target'),
    active: 0,
    replacement: {value: 'changed'},
    beforeReplace: () => { guardCalled = true; return false; },
    host: {querySelector: () => control},
    onStaged() {}, refresh() {}, location: () => 'P1'
  };
  FindBar.prototype.replace.call(bar, true);
  assert.equal(guardCalled, true);
  assert.equal(model.patch().operations.length, 0);
});
