const {test} = require('node:test');
const assert = require('node:assert/strict');
const {OutputReveal} = require('../output-reveal.cjs');

test('reveal rejects empty state and renderer paths and reveals only the saved output', () => {
  const revealed = [];
  const reveal = new OutputReveal(file => revealed.push(file));
  assert.throws(() => reveal.show(), /No saved copy/);
  reveal.saved('/private/tmp/copy.hwpx');
  assert.throws(() => reveal.show('/private/tmp/other.hwpx'), /arguments/);
  reveal.show();
  assert.deepEqual(revealed, ['/private/tmp/copy.hwpx']);
  reveal.clear();
  assert.throws(() => reveal.show(), /No saved copy/);
});
