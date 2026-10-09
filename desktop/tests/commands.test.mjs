import {test} from 'node:test';
import assert from 'node:assert/strict';
import {fuzzyScore, matchingCommands} from '../commands.js';

test('palette matches English and Korean labels and ranks exact matches first', () => {
  const commands = [{id: 'open', en: 'Open…', ko: '문서 열기'}, {id: 'outline', en: 'Outline', ko: '개요'}];
  assert.equal(matchingCommands(commands, '열기')[0].id, 'open');
  assert.equal(matchingCommands(commands, 'op')[0].id, 'open');
  assert.equal(matchingCommands(commands, 'otl')[0].id, 'outline');
  assert.equal(matchingCommands(commands, 'missing').length, 0);
  assert.ok(fuzzyScore('Open', 'open') > fuzzyScore('Outline pane', 'op'));
});

test('running a palette result leaves focus with the command or proof slip', async () => {
  const {CommandPalette} = await import('../palette.js');
  let restore;
  let selected;
  const match = {node: {id: 'n'}};
  CommandPalette.prototype.run.call({
    items: [{match}], selected: 0,
    close: value => { restore = value; },
    onText: value => { selected = value; }
  });
  assert.equal(restore, false);
  assert.equal(selected, match);
});
