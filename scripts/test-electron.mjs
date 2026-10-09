import {_electron as electron} from 'playwright';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createRequire} from 'node:module';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);
const {serveAppRequest} = require('../desktop/protocol.cjs');
const temporary = await fs.mkdtemp(path.join(root, 'work', 'electron-test-'));
let application;
try {
  application = await electron.launch({args: [root], cwd: root, timeout: 30000});
  await application.evaluate(({app}, userData) => app.setPath('userData', userData), temporary);
  const window = await application.firstWindow();
  const errors = [];
  window.on('pageerror', error => errors.push(error.message));
  await window.waitForSelector('#emptyOpen');
  assert.equal(await window.locator('#empty p').count(), 0);
  assert.equal(await window.locator('#empty .drop').count(), 1);
  assert.equal(await window.locator('#empty .ctas button').count(), 2);
  assert.equal(await window.evaluate(() => Object.isFrozen(window.hwp)), true);
  await window.locator('#demo').click();
  await window.waitForSelector('#workspace:not([hidden])');
  // The generated sample is a preservation fixture. It may not yet render in
  // rhwp; qualify page interactions against the supplied real public sample.
  await window.waitForFunction(() => document.querySelector('.page-image') || !document.getElementById('structureChip').hidden);
  if (await window.locator('.page-image').count() === 0) {
    console.log('Generated sample: structure fallback (rhwp qualification pending)');
  }
  const publicFile = path.join(root, 'fixtures', 'public', 'hwp-cli-report-tables.hwpx');
  await fs.access(publicFile);
  await application.evaluate(({dialog}, file) => {
    dialog.showOpenDialog = async () => ({canceled: false, filePaths: [file]});
  }, publicFile);
  await window.locator('#open').click();
  await window.waitForSelector('.page-image');
  await window.waitForFunction(() => {
    const image = document.querySelector('.page-image');
    return image.complete && image.naturalWidth > 0;
  });
  const hit = window.locator('.text-hit[data-node-id]:not([data-node-id=""])').first();
  await hit.click();
  await window.waitForSelector('.slip');
  const original = await window.locator('#textEdit').inputValue();
  await window.locator('#textEdit').fill(original + ' 검토');
  await window.locator('#applyText').click();
  assert.match(await window.locator('#dirty').innerText(), /1/);
  assert.equal(await window.locator('#changes .change').count(), 1);
  assert.match(await window.locator('#changes ins').innerText(), /검토/);
  assert.ok(await window.locator('.mark-line.human').count() > 0);
  await window.keyboard.press(process.platform === 'darwin' ? 'Meta+z' : 'Control+z');
  assert.equal(await window.locator('#changes .change').count(), 0);
  await window.keyboard.press(process.platform === 'darwin' ? 'Meta+Shift+z' : 'Control+Shift+z');
  assert.equal(await window.locator('#changes .change').count(), 1);
  const output = path.join(temporary, 'review-copy.hwpx');
  await application.evaluate(({dialog}, file) => {
    dialog.showSaveDialog = async () => ({canceled: false, filePath: file});
  }, output);
  await window.locator('#save').click();
  await window.waitForSelector('#reviewDialog[open]');
  assert.equal(await window.locator('#confirmSave').isDisabled(), true);
  await window.locator('#acknowledge').check();
  await window.locator('#confirmSave').click();
  await window.waitForFunction(() => !document.getElementById('reviewDialog').open);
  assert.ok((await fs.stat(output)).size > 0);
  // The save path must still fail closed when the dialog chooses this file again.
  const savedBytes = await fs.readFile(output);
  await window.locator('#save').click();
  await window.locator('#acknowledge').check();
  await window.locator('#confirmSave').click();
  await window.waitForFunction(() => document.getElementById('status').classList.contains('error'));
  assert.deepEqual(await fs.readFile(output), savedBytes);
  await window.locator('#closeReview').click();
  await window.locator('#changes .change .acts button').click();
  await window.locator('#language').click();
  const language = await window.evaluate(() => document.documentElement.lang);
  await window.reload();
  assert.equal(await window.evaluate(() => document.documentElement.lang), language);
  for (const url of ['app://workbench/../package.json', 'app://workbench/unknown.xyz']) {
    const response = await serveAppRequest({url, method: 'GET'}, {
      desktop: path.join(root, 'desktop'), vendor: path.join(root, 'node_modules', '@rhwp', 'core')
    });
    assert.notEqual(response.status, 200, url);
  }
  const forbidden = await window.evaluate(async () => (await fetch('app://workbench/unknown.xyz')).status);
  assert.notEqual(forbidden, 200);
  assert.deepEqual(errors, []);
  console.log(`Electron checks passed: ${process.platform}/${process.arch}; real public HWPX page → edit → undo/redo → native engine save; no overwrite; protocol rejection; language persistence`);
} finally {
  if (application) {
    await application.evaluate(({app}) => app.exit(0)).catch(() => {});
    await application.close().catch(() => {});
  }
  await fs.rm(temporary, {recursive: true, force: true});
}
