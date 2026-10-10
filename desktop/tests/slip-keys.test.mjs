import {test} from 'node:test';
import assert from 'node:assert/strict';
import {ProofSlip} from '../slip.js';
import {textError} from '../edit-model.js';

test('marker validation keeps original markers and refuses invented or duplicate markers', () => {
  assert.equal(textError('한\ufdd1글', '\ufdd0\ufdd1', true), null);
  assert.equal(textError('한\ufdd2글', '\ufdd0\ufdd1', true), 'inlineMarker');
  assert.equal(textError('\ufdd0\ufdd0', '\ufdd0', true), 'inlineMarker');
});
function key(modifiers) {
  let prevented = false;
  return {key: 'Enter', code: 'Enter', ...modifiers, preventDefault: () => { prevented = true; },
    stopPropagation() {}, get prevented() { return prevented; }};
}
test('plain Enter is blocked; Option Return and composition reach the Korean IME', () => {
  const slip = new ProofSlip(null, () => {}, () => {});
  for (const modifiers of [{altKey: true}, {shiftKey: true}, {isComposing: true}]) {
    const event = key(modifiers);
    slip.keydown(event);
    assert.equal(event.prevented, false);
  }
  const plain = key({});
  slip.keydown(plain);
  assert.equal(plain.prevented, true);
});
test('command Enter composition replay queues stage only once', () => {
  const slip = new ProofSlip(null, () => {}, () => {});
  slip.keydown(key({metaKey: true, isComposing: true}));
  assert.equal(slip.stageAfterComposition, true);
});

function editorSlip(t) {
  const timers = new Map();
  let timerId = 0;
  t.mock.method(globalThis, 'setTimeout', callback => {
    timers.set(++timerId, callback);
    return timerId;
  });
  t.mock.method(globalThis, 'clearTimeout', id => timers.delete(id));
  const makeElement = () => ({
    children: [], style: {}, scrollHeight: 54,
    append(...children) { this.children.push(...children); }, setAttribute() {}, focus() {},
    replaceChildren(...children) { this.children = children; },
    addEventListener(name, callback) { this[name] = callback; },
    dispatch(name) { this[name]?.(); },
    parentElement: {focus() {}}
  });
  const originalDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  globalThis.document = {createElement: makeElement, createTextNode: text => ({textContent: text})};
  t.after(() => {
    if (originalDocument) Object.defineProperty(globalThis, 'document', originalDocument);
    else delete globalThis.document;
  });
  const staged = [];
  const slip = new ProofSlip(makeElement(), (_node, value) => {
    staged.push(value);
    return true;
  }, () => {});
  slip.update = () => {};
  slip.relabel = () => {};
  slip.position = () => {};
  const open = () => slip.open({id: 'one', editable: true}, 'before', 'hwpx', {}, true);
  open();
  const flush = () => {
    const pending = [...timers.values()];
    timers.clear();
    for (const callback of pending) callback();
  };
  return {slip, staged, flush, open};
}

test('keyCode 229 Enter reaches the WebKit IME even when isComposing is false', () => {
  const slip = new ProofSlip(null, () => {}, () => {});
  const event = key({keyCode: 229, isComposing: false});
  slip.keydown(event);
  assert.equal(event.prevented, false);
});

test('Chromium command Enter stages once after compositionend and committed input', t => {
  const {slip, staged, flush} = editorSlip(t);
  const editor = slip.editor;
  editor.dispatch('compositionstart');
  slip.keydown(key({metaKey: true, isComposing: true, keyCode: 229}));
  flush();
  assert.deepEqual(staged, []);
  editor.dispatch('compositionend');
  editor.value = '한글';
  flush();
  assert.deepEqual(staged, ['한글']);
  flush();
  assert.deepEqual(staged, ['한글']);
});

test('WebKit command Enter after compositionend stages the committed text once', t => {
  const {slip, staged, flush} = editorSlip(t);
  const editor = slip.editor;
  editor.dispatch('compositionstart');
  editor.dispatch('compositionend');
  const event = key({metaKey: true, isComposing: false, keyCode: 229});
  slip.keydown(event);
  assert.equal(event.prevented, false);
  assert.deepEqual(staged, []);
  editor.value = '한글';
  flush();
  assert.deepEqual(staged, ['한글']);
});

test('closing or reopening a slip cancels pending composition staging', t => {
  const {slip, staged, flush, open} = editorSlip(t);
  slip.editor.dispatch('compositionstart');
  slip.keydown(key({metaKey: true, isComposing: true}));
  slip.editor.dispatch('compositionend');
  slip.close(false);
  open();
  slip.editor.value = 'other edit';
  flush();
  assert.deepEqual(staged, []);
});

test('marker display preserves textarea value and Korean composition beside markers', t => {
  const {slip, staged, flush} = editorSlip(t);
  const original = '앞\ufdd0뒤';
  slip.open({id: 'one', text: original, editable: true}, original, 'hwpx', {}, true);
  ProofSlip.prototype.update.call(slip);
  assert.equal(slip.editor.value, original);
  assert.equal(slip.markerMirror.children.filter(child => child.className === 'inline-marker').length, 1);
  slip.editor.selectionStart = 2;
  slip.editor.selectionEnd = 2;
  slip.editor.dispatch('compositionstart');
  slip.keydown(key({metaKey: true, isComposing: true, keyCode: 229}));
  assert.deepEqual(staged, []);
  slip.editor.value = '앞\ufdd0한글뒤';
  slip.editor.dispatch('compositionend');
  ProofSlip.prototype.update.call(slip);
  assert.equal(slip.editor.selectionStart, 2);
  assert.equal(slip.editor.selectionEnd, 2);
  flush();
  assert.deepEqual(staged, ['앞\ufdd0한글뒤']);
});
