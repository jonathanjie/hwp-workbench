const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const {RecentStore, validateDroppedFile} = require('../recent-store.cjs');

const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aWQAAAABJRU5ErkJggg==', 'base64');

test('recent files persist eight opaque ids and reopen only known regular files', async () => {
  const directory = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-recent-')));
  try {
    const store = new RecentStore(path.join(directory, 'data'));
    for (let i = 0; i < 9; i += 1) {
      const file = path.join(directory, `${i}.hwpx`);
      await fs.writeFile(file, 'sample');
      await store.record(file, 'hwpx');
    }
    const list = await store.list();
    assert.equal(list.length, 8);
    assert.deepEqual(Object.keys(list[0]).sort(), ['format', 'id', 'name', 'openedAt', 'thumbnail']);
    assert.equal(list[0].name, '8.hwpx');
    const reloaded = new RecentStore(path.join(directory, 'data'));
    assert.equal(await reloaded.fileFor(list[0].id), path.join(directory, '8.hwpx'));
    assert.equal(await reloaded.record(path.join(directory, '8.hwpx'), 'hwpx'), list[0].id);
    await reloaded.cacheThumbnail(list[0].id, `data:image/png;base64,${png.toString('base64')}`);
    assert.match((await reloaded.list())[0].thumbnail, /^data:image\/png;base64,/);
    await assert.rejects(reloaded.cacheThumbnail('../outside', `data:image/png;base64,${png.toString('base64')}`));
    await assert.rejects(reloaded.cacheThumbnail(list[0].id, 'data:image/svg+xml,evil'));
    await assert.rejects(reloaded.cacheThumbnail(list[0].id, 'data:image/png;base64,' + 'a'.repeat(300000)));
    await fs.rm(path.join(directory, '8.hwpx'));
    await assert.rejects(reloaded.fileFor(list[0].id));
    await assert.rejects(reloaded.fileFor(0));
  } finally {
    await fs.rm(directory, {recursive: true, force: true});
  }
});

test('drop rejects wrong extensions, directories and symlinks', async () => {
  const directory = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-drop-')));
  try {
    const file = path.join(directory, 'doc.HWPX');
    await fs.writeFile(file, 'sample');
    assert.equal(await validateDroppedFile(file), file);
    await fs.writeFile(path.join(directory, 'doc.txt'), 'sample');
    await assert.rejects(validateDroppedFile(path.join(directory, 'doc.txt')), /Unsupported/);
    await fs.mkdir(path.join(directory, 'folder.hwp'));
    await assert.rejects(validateDroppedFile(path.join(directory, 'folder.hwp')), /regular/);
    await fs.symlink(file, path.join(directory, 'link.hwp'));
    await assert.rejects(validateDroppedFile(path.join(directory, 'link.hwp')), /regular/);
  } finally {
    await fs.rm(directory, {recursive: true, force: true});
  }
});

test('recent store and drops accept symlinked parent folders but not a symlinked leaf', async () => {
  const directory = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-recent-link-')));
  try {
    const real = path.join(directory, 'real');
    await fs.mkdir(real);
    const linked = path.join(directory, 'linked');
    await fs.symlink(real, linked);
    const file = path.join(real, 'doc.hwpx');
    await fs.writeFile(file, 'sample');

    // Regression: a profile under a symlinked parent (macOS /var, /tmp) refused all recents.
    const store = new RecentStore(path.join(linked, 'profile'));
    const id = await store.record(file, 'hwpx');
    assert.equal((await store.list())[0].id, id);

    // A dropped path through a symlinked parent opens as its canonical path.
    assert.equal(await validateDroppedFile(path.join(linked, 'doc.hwpx')), file);

    // The store folder itself must not be a symlink.
    await fs.mkdir(path.join(directory, 'elsewhere'));
    await fs.symlink(path.join(directory, 'elsewhere'), path.join(directory, 'store-link'));
    await assert.rejects(new RecentStore(path.join(directory, 'store-link')).list(), /changed/);
  } finally {
    await fs.rm(directory, {recursive: true, force: true});
  }
});
