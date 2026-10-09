import {test} from 'node:test';
import assert from 'node:assert/strict';

function radio(scheme) {
  return {
    dataset: {scheme},
    attributes: {},
    classList: {toggle() {}},
    setAttribute(key, value) { this.attributes[key] = value; },
    focus() { this.focused = true; }
  };
}

test('scheme choices use one tab stop and select with click, arrows, Home and End', async () => {
  const {KeySchemeSelector} = await import('../key-scheme.js');
  const buttons = [radio('windows'), radio('mac')];
  const changes = [];
  const selector = new KeySchemeSelector({querySelectorAll: () => buttons}, scheme => changes.push(scheme));
  selector.setScheme('windows');
  assert.equal(buttons[0].attributes['aria-checked'], 'true');
  assert.equal(buttons[0].tabIndex, 0);
  assert.equal(buttons[1].tabIndex, -1);
  buttons[1].onclick();
  assert.equal(changes.at(-1), 'mac');
  assert.equal(buttons[1].attributes['aria-checked'], 'true');
  for (const [key, expected] of [['ArrowRight', 0], ['ArrowLeft', 1], ['ArrowUp', 0], ['ArrowDown', 1], ['Home', 0], ['End', 1]]) {
    let prevented = false;
    const current = buttons.find(button => button.tabIndex === 0);
    current.onkeydown({key, preventDefault() { prevented = true; }});
    assert.equal(prevented, true);
    assert.equal(changes.at(-1), buttons[expected].dataset.scheme);
    assert.equal(buttons[expected].focused, true);
    assert.equal(buttons.filter(button => button.tabIndex === 0).length, 1);
  }
});

test('scheme control leaves Tab and system shortcuts alone', async () => {
  const {KeySchemeSelector} = await import('../key-scheme.js');
  const buttons = [radio('windows'), radio('mac')];
  const changes = [];
  new KeySchemeSelector({querySelectorAll: () => buttons}, scheme => changes.push(scheme));
  const preventDefault = () => assert.fail('unrelated key intercepted');
  buttons[0].onkeydown({key: 'Tab', preventDefault});
  buttons[0].onkeydown({key: 'ArrowLeft', metaKey: true, preventDefault});
  assert.deepEqual(changes, []);
});
