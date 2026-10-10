import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {fetchFixtures} from '../../scripts/fetch-public-fixtures.mjs';

const data = Buffer.from('fixed native fixture bytes');
const digest = algorithm => createHash(algorithm).update(data).digest('hex');
const item = {
  name: 'sample.hwp', repository: 'owner/repo', bytes: data.length,
  blobSha1: createHash('sha1').update(`blob ${data.length}\0`).update(data).digest('hex'),
  sha256: digest('sha256')
};
async function folder(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-download-test-'));
  t.after(() => fs.rm(root, {recursive: true, force: true}));
  return root;
}
const response = () => new Response(JSON.stringify({encoding: 'base64', content: data.toString('base64')}));

test('downloads pinned bytes and reports SHA-256; existing files need no download', async t => {
  const root = await folder(t);
  const result = await fetchFixtures({manifest: {fixtures: [item]}, folder: root, fetchBlob: response});
  assert.deepEqual(result, [{name: item.name, status: 'verified', sha256: item.sha256, bytes: data.length}]);
  assert.deepEqual(await fs.readFile(path.join(root, item.name)), data);
  assert.deepEqual(await fetchFixtures({manifest: {fixtures: [item]}, folder: root,
    fetchBlob: () => assert.fail('existing fixture must not be downloaded')}), result);
});

test('refuses wrong size, Git blob identity, SHA-256 and encoding without publication', async t => {
  const root = await folder(t);
  for (const changes of [{bytes: data.length + 1}, {blobSha1: '0'.repeat(40)}, {sha256: '0'.repeat(64)}]) {
    const [result] = await fetchFixtures({manifest: {fixtures: [{...item, ...changes}]}, folder: root, fetchBlob: response});
    assert.equal(result.status, 'download-failed');
    await assert.rejects(fs.stat(path.join(root, item.name)), {code: 'ENOENT'});
  }
  const [result] = await fetchFixtures({manifest: {fixtures: [item]}, folder: root,
    fetchBlob: () => new Response(JSON.stringify({encoding: 'text', content: 'wrong'}))});
  assert.equal(result.status, 'download-failed');
  assert.deepEqual(await fs.readdir(root), []);
});

test('preserves existing invalid files, symlink destinations and concurrent publication', async t => {
  const root = await folder(t);
  const destination = path.join(root, item.name);
  await fs.writeFile(destination, 'user bytes');
  const settings = {manifest: {fixtures: [item]}, folder: root, fetchBlob: response};
  assert.equal((await fetchFixtures(settings))[0].status, 'download-failed');
  assert.equal(await fs.readFile(destination, 'utf8'), 'user bytes');
  await fs.unlink(destination);
  await fs.symlink('missing', destination);
  assert.equal((await fetchFixtures(settings))[0].status, 'download-failed');
  assert.equal(await fs.readlink(destination), 'missing');
  await fs.unlink(destination);
  settings.fetchBlob = async () => {
    await fs.writeFile(destination, 'concurrent bytes');
    return response();
  };
  assert.equal((await fetchFixtures(settings))[0].status, 'download-failed');
  assert.equal(await fs.readFile(destination, 'utf8'), 'concurrent bytes');
  assert.deepEqual(await fs.readdir(root), [item.name]);
});

test('refuses traversal, network failures and oversized responses', async t => {
  const root = await folder(t);
  const [unsafe] = await fetchFixtures({manifest: {fixtures: [{...item, name: '../escape.hwp'}]}, folder: root,
    fetchBlob: () => assert.fail('unsafe name must fail before download')});
  assert.equal(unsafe.status, 'download-failed');
  for (const fetchBlob of [() => new Response('unavailable', {status: 503}),
    () => new Response('x'.repeat(4 * 1024 * 1024 + 1)), () => {throw Error('network unavailable');}]) {
    assert.equal((await fetchFixtures({manifest: {fixtures: [item]}, folder: root, fetchBlob}))[0].status, 'download-failed');
  }
  assert.deepEqual(await fs.readdir(root), []);
});
