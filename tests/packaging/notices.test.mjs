import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
const notices = () => import('../../scripts/third-party-notices.mjs');
test('license allowlist reports every unknown or absent expression', async () => {
  const {checkLicenses} = await notices();
  assert.throws(() => checkLicenses([{name: 'one', version: '1', license: 'Secret-1.0'}, {name: 'two', version: '2', license: null}]), error => /one 1.*Secret-1.0/s.test(error.message) && /two 2/s.test(error.message));
});
test('normal closure excludes build and dev dependencies but includes their normal overlap', async () => {
  const {normalPackages} = await notices();
  const metadata = {workspace_members: ['app'], packages: ['app','runtime','dev','build'].map(id => ({id, name: id === 'app' ? 'hwp-workbench-app' : id})), resolve: {nodes: [
    {id: 'app', deps: [{pkg: 'runtime', dep_kinds: [{kind: null}]}, {pkg: 'dev', dep_kinds: [{kind: 'dev'}]}, {pkg: 'build', dep_kinds: [{kind: 'build'}]}]},
    ...['runtime','dev','build'].map(id => ({id, deps: []}))
  ]}};
  assert.deepEqual(normalPackages(metadata).map(p => p.name), ['runtime']);
});
test('missing license file uses marked standard SPDX text; output order is deterministic', async t => {
  const {renderNotices} = await notices();
  const root = await fs.mkdtemp(path.join(await fs.realpath(os.tmpdir()), 'hwp-notices-test-'));
  t.after(() => fs.rm(root, {recursive: true, force: true}));
  const packages = ['zeta', 'alpha'].map(name => ({name, version: '1.0.0', license: 'MIT', repository: 'https://example.org/source', manifest_path: path.join(root, 'Cargo.toml')}));
  const forward = await renderNotices(packages);
  assert.equal(forward, await renderNotices(packages.reverse()));
  assert.match(forward, /standard SPDX text/i);
  assert.match(forward, /Permission is hereby granted/);
  assert.ok(forward.indexOf('alpha 1.0.0') < forward.indexOf('zeta 1.0.0'));
});
test('MPL notice links the exact version source archive', async t => {
  const {renderNotices} = await notices();
  const root = await fs.mkdtemp(path.join(await fs.realpath(os.tmpdir()), 'hwp-mpl-test-'));
  t.after(() => fs.rm(root, {recursive: true, force: true}));
  await fs.writeFile(path.join(root, 'LICENSE'), 'MPL license text');
  const result = await renderNotices([{name: 'selectors', version: '0.36.1', license: 'MPL-2.0', repository: 'https://example.org', manifest_path: path.join(root, 'Cargo.toml')}]);
  assert.match(result, /https:\/\/crates.io\/api\/v1\/crates\/selectors\/0.36.1\/download/);
  assert.match(result, /3.2/);
});
test('missing license texts report every package without a supported fallback', async t => {
  const {renderNotices} = await notices();
  const root = await fs.mkdtemp(path.join(await fs.realpath(os.tmpdir()), 'hwp-license-missing-test-'));
  t.after(() => fs.rm(root, {recursive: true, force: true}));
  const packages = ['alpha', 'beta'].map(name => ({name, version: '1', license: 'Unicode-3.0', manifest_path: path.join(root, 'Cargo.toml')}));
  await assert.rejects(renderNotices(packages), error => /alpha 1/.test(error.message) && /beta 1/.test(error.message));
});
test('license inventory retains all license directory files and notices', async t => {
  const {renderNotices} = await notices();
  const root = await fs.mkdtemp(path.join(await fs.realpath(os.tmpdir()), 'hwp-license-directory-test-'));
  t.after(() => fs.rm(root, {recursive: true, force: true}));
  await fs.mkdir(path.join(root, 'licenses'));
  await fs.writeFile(path.join(root, 'licenses/MIT.txt'), 'upstream license and copyright');
  await fs.writeFile(path.join(root, 'NOTICE'), 'upstream notice');
  const result = await renderNotices([{name: 'example', version: '1', license: 'MIT', manifest_path: path.join(root, 'Cargo.toml')}]);
  assert.match(result, /upstream license and copyright/);
  assert.match(result, /upstream notice/);
  assert.doesNotMatch(result, /No license text was shipped/);
});
test('NOTICE alone does not replace the license text', async t => {
  const {renderNotices} = await notices();
  const root = await fs.mkdtemp(path.join(await fs.realpath(os.tmpdir()), 'hwp-notice-only-test-'));
  t.after(() => fs.rm(root, {recursive: true, force: true}));
  await fs.writeFile(path.join(root, 'NOTICE'), 'Attribution only');
  const result = await renderNotices([{name: 'example', version: '1', license: 'MIT', manifest_path: path.join(root, 'Cargo.toml')}]);
  assert.match(result, /Attribution only/);
  assert.match(result, /Standard SPDX text/);
});
