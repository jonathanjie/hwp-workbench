const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const {openArgument, requestedOpen, OpenHandoff} = require('../open-handoff.cjs');

test('handoff accepts only absolute regular native files and retains staged work on cancellation', async () => {
  const root = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-open-')));
  try {
    const file = path.join(root, 'review.hwpx');
    await fs.writeFile(file, 'document');
    assert.equal(openArgument(['electron', '.', '--open', file]), file);
    assert.equal(openArgument(['--open', 'relative.hwpx']), null);
    assert.equal(openArgument(['--open', file, '--open', file]), null);
    let staged = true;
    let approved = false;
    const loaded = [];
    let focused = 0;
    const handoff = new OpenHandoff({hasStaged: () => staged, confirm: async () => approved,
      load: async file => { loaded.push(file); return {document: 'opened'}; },
      complete: () => {}, focus: () => { focused += 1; }});
    assert.equal(await handoff.open(file), false);
    assert.equal(loaded.length, 0);
    approved = true;
    assert.equal(await handoff.open(file), true);
    assert.deepEqual(loaded, [file]);
    staged = false;
    const link = path.join(root, 'link.hwpx');
    await fs.symlink(file, link);
    await assert.rejects(handoff.open(link), /regular|symlink/i);
    await assert.rejects(handoff.open(root), /Unsupported/i);
    await assert.rejects(handoff.open(path.join(root, 'missing.hwp')), /ENOENT/);
    assert.ok(focused >= 2);
  } finally {
    await fs.rm(root, {recursive: true, force: true});
  }
});

test('second-instance open uses the lock data before reordered argv', () => {
  const file = '/Users/someone/Documents/form.hwpx';
  // Regression: Chromium moved switches before paths in second-instance argv,
  // so the file no longer followed --open and the handoff was ignored.
  const reordered = ['Electron', '--user-data-dir=/tmp/profile', '--open', '/repo', file];
  assert.equal(openArgument(reordered), '/repo');
  assert.equal(requestedOpen(reordered, {open: file}), file);
  assert.equal(requestedOpen(['Electron', '--open', file], {open: null}), file);
  assert.equal(requestedOpen(['Electron'], {open: 'relative.hwpx'}), null);
});
