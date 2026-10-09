const {test} = require('node:test');
const assert = require('node:assert/strict');
const {LatestPreview} = require('../preview-lane.cjs');

test('new preview aborts old work and drops its result', async () => {
  const lane = new LatestPreview();
  let release;
  let signal;
  const first = lane.run(async abort => {
    signal = abort;
    return new Promise(resolve => { release = resolve; });
  });
  const second = lane.run(async () => 'new');
  assert.equal(signal.aborted, true);
  release('old');
  assert.equal(await first, null);
  assert.equal(await second, 'new');
});
