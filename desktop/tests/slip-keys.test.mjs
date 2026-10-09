import {test} from 'node:test';
import assert from 'node:assert/strict';
import {ProofSlip} from '../slip.js';
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
