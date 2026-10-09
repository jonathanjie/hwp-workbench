import {test} from 'node:test';
import assert from 'node:assert/strict';
import {KEYMAP, validateKeymap, Keymap} from '../keymap.js';
const event = (code, modifiers = {}) => ({code, ...modifiers});

test('table has valid keys and no collisions in either scheme', () => {
  assert.deepEqual(validateKeymap(KEYMAP), []);
});
test('physical keys cover both schemes, composing keys and system keys', () => {
  const keys = new Keymap();
  assert.equal(keys.match(event('KeyO', {altKey: true})), 'open');
  assert.equal(keys.match(event('KeyF', {ctrlKey: true})), 'find');
  assert.equal(keys.match(event('KeyF', {metaKey: true})), 'find');
  assert.equal(keys.match(event('KeyA', {ctrlKey: true})), 'selectAll');
  assert.equal(keys.match(event('KeyF', {ctrlKey: true, isComposing: true})), null);
  for (const code of ['KeyQ', 'KeyW', 'KeyH', 'Tab', 'Space']) {
    assert.equal(keys.match(event(code, {metaKey: true})), null);
  }
  assert.equal(keys.match(event('Space', {ctrlKey: true})), null);
  keys.scheme = 'mac';
  assert.equal(keys.match(event('KeyO', {altKey: true})), null);
  assert.equal(keys.match(event('KeyF', {ctrlKey: true})), null);
  assert.equal(keys.match(event('KeyF', {metaKey: true})), 'find');
});
test('chord pending, Escape and 1500ms timeout', () => {
  let expire;
  const pending = [];
  const keys = new Keymap({onPending: value => pending.push(value),
    setTimer: (fn, delay) => { assert.equal(delay, 1500); expire = fn; return 1; }, clearTimer: () => {}});
  assert.equal(keys.match(event('KeyG', {ctrlKey: true})), 'pending');
  assert.equal(keys.match(event('KeyC')), 'controlMarks');
  keys.match(event('KeyG', {metaKey: true}));
  expire();
  assert.equal(keys.match(event('KeyC')), null);
  keys.match(event('KeyG', {ctrlKey: true}));
  assert.equal(keys.match(event('Escape')), 'cancelChord');
  assert.equal(pending.at(-1), '');
});

test('table validator rejects duplicate bindings and repeated modifiers', () => {
  const row = {id: 'one', mac: ['Meta+KeyO'], windows: ['Ctrl+KeyO']};
  assert.ok(validateKeymap([row, {...row, id: 'two'}]).some(message => message.includes('Duplicate')));
  assert.ok(validateKeymap([{id: 'bad', mac: ['Meta+Meta+KeyO'], windows: []}]).some(message => message.includes('Invalid')));
});

test('chord timers are called unbound, as browsers require', () => {
  // Regression: `this.setTimer(...)` threw "Illegal invocation" in Chromium,
  // so ⌃G then C never toggled 조판 부호. Strict functions see `this` here.
  function strictSetTimer(callback, delay) {
    assert.equal(this, undefined);
    return 1;
  }
  function strictClearTimer() {
    assert.equal(this, undefined);
  }
  const keys = new Keymap({scheme: 'windows', setTimer: strictSetTimer, clearTimer: strictClearTimer});
  const press = (code, ctrlKey = false) => keys.match({code, ctrlKey, metaKey: false, altKey: false, shiftKey: false, isComposing: false});
  assert.equal(press('KeyG', true), 'pending');
  assert.equal(press('KeyC'), 'controlMarks');
});
