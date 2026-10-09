import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import vm from 'node:vm';

const builderUrl = new URL('../../scripts/build-frontend.mjs', import.meta.url);
const builderExists = await fs.stat(builderUrl).then(() => true).catch(() => false);

async function buildFixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-frontend-'));
  t.after(() => fs.rm(root, {recursive: true, force: true}));
  const desktopRoot = path.join(root, 'desktop');
  const vendorRoot = path.join(root, 'vendor');
  const outputRoot = path.join(root, 'dist');
  const files = {
    'desktop/index.html': '<html><script type="module" src="renderer.js"></script></html>',
    'desktop/renderer.js': 'renderer', 'desktop/tauri-bridge.js': 'bridge',
    'desktop/styles/app.css': 'styles', 'desktop/assets/logo.svg': '<svg/>',
    'desktop/assets/icon.png': 'icon', 'desktop/main.cjs': 'secret backend',
    'desktop/agent-files.py': 'backend', 'desktop/package.json': '{}',
    'desktop/tests/private.test.js': 'test', 'desktop/assets/font.woff2': 'font',
    'desktop/assets/sample.hwpx': 'document', 'vendor/rhwp.js': 'runtime',
    'vendor/rhwp_bg.wasm': 'wasm', 'vendor/rhwp.d.ts': 'types',
    'vendor/README.md': 'readme', 'dist/stale.js': 'old'
  };
  for (const [name, contents] of Object.entries(files)) {
    const file = path.join(root, name);
    await fs.mkdir(path.dirname(file), {recursive: true});
    await fs.writeFile(file, contents);
  }
  assert.ok(builderExists, 'frontend build script must exist');
  const {buildFrontend} = await import(builderUrl);
  return {buildFrontend, desktopRoot, vendorRoot, outputRoot, root};
}

test('frontend build copies only renderer assets and rhwp runtime and injects bridge first', async t => {
  const fixture = await buildFixture(t);
  await fixture.buildFrontend(fixture);
  const paths = (await fs.readdir(fixture.outputRoot, {recursive: true}))
    .filter(name => path.extname(name)).sort();
  assert.deepEqual(paths, ['assets/icon.png', 'assets/logo.svg', 'index.html', 'renderer.js',
    'styles/app.css', 'tauri-bridge.js', 'vendor/rhwp/rhwp.js', 'vendor/rhwp/rhwp_bg.wasm']);
  const html = await fs.readFile(path.join(fixture.outputRoot, 'index.html'), 'utf8');
  assert.ok(html.includes('<script src="tauri-bridge.js"></script>'));
  assert.ok(html.indexOf('src="tauri-bridge.js"') < html.indexOf('type="module"'));
  assert.equal(await fs.readFile(path.join(fixture.outputRoot, 'vendor/rhwp/rhwp_bg.wasm'), 'utf8'), 'wasm');
});

test('frontend build refuses source symlinks instead of bundling outside bytes', async t => {
  const fixture = await buildFixture(t);
  await fs.writeFile(path.join(fixture.root, 'secret.js'), 'secret');
  await fs.symlink(path.join(fixture.root, 'secret.js'), path.join(fixture.desktopRoot, 'escape.js'));
  await assert.rejects(fixture.buildFrontend(fixture), /symlink/i);
});

test('rhwp vendor URL resolves relative to the document in both shells', async () => {
  const source = await fs.readFile(new URL('../page-view.js', import.meta.url), 'utf8');
  const declaration = source.match(/const VENDOR = ([^;]+);/)[1];
  for (const baseURI of ['app://workbench/index.html', 'tauri://localhost/index.html', 'http://tauri.localhost/index.html']) {
    const actual = vm.runInNewContext(declaration.replace('import.meta.url', JSON.stringify('file:///desktop/page-view.js')), {URL, document: {baseURI}});
    assert.equal(actual, new URL('vendor/rhwp/', baseURI).href);
  }
});
