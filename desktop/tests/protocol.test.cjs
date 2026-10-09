const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');

test('protocol confines files, rejects traversal and outside symlinks, and serves wasm MIME', async () => {
  const {serveAppRequest} = require('../protocol.cjs');
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-protocol-'));
  try {
    const desktop = path.join(root, 'desktop');
    const vendor = path.join(root, 'vendor');
    await fs.mkdir(desktop);
    await fs.mkdir(vendor);
    await fs.writeFile(path.join(desktop, 'index.html'), 'shell');
    await fs.writeFile(path.join(vendor, 'rhwp_bg.wasm'), 'wasm');
    await fs.writeFile(path.join(root, 'secret.js'), 'secret');
    await fs.symlink(path.join(root, 'secret.js'), path.join(desktop, 'escape.js'));
    const request = (url) => serveAppRequest({url, method: 'GET'}, {desktop, vendor});
    assert.equal((await request('app://workbench/index.html')).status, 200);
    const wasm = await request('app://workbench/vendor/rhwp/rhwp_bg.wasm');
    assert.equal(wasm.headers.get('content-type'), 'application/wasm');
    for (const url of [
      'app://workbench/../package.json',
      'app://workbench/%2e%2e/secret.js',
      'app://workbench/%2e%2e%2fsecret.js',
      'app://workbench/escape.js',
      'app://workbench/unknown.xyz',
      'app://elsewhere/index.html',
      'app://workbench/main.cjs',
      'app://workbench/vendor/other/rhwp.js'
    ]) {
      assert.notEqual((await request(url)).status, 200, url);
    }
  } finally {
    await fs.rm(root, {recursive: true, force: true});
  }
});
