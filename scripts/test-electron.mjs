import {_electron as electron} from 'playwright';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createRequire} from 'node:module';
import {spawnSync} from 'node:child_process';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);
const {serveAppRequest} = require('../desktop/protocol.cjs');
const temporary = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-electron-')));
const modifier = process.platform === 'darwin' ? 'Meta' : 'Control';
const skipped = [];
const checks = [];
let application;
let window;

async function shortcut(keys) {
  await window.keyboard.press(`${modifier}+${keys}`);
}

async function command(query) {
  await shortcut('k');
  await window.locator('#paletteQuery').fill(query);
  await window.keyboard.press('Enter');
}

async function waitDocument(name) {
  await window.waitForFunction(name => {
    const image = document.querySelector('.page-image');
    const structure = document.querySelector('.structure-page');
    return document.getElementById('filename').textContent === name && !document.getElementById('demo').disabled &&
      ((image?.complete && image.naturalWidth > 0) || !!structure);
  }, name);
}

async function openFile(file) {
  await application.evaluate(({dialog}, file) => {
    dialog.showOpenDialog = async () => ({canceled: false, filePaths: [file]});
  }, file);
  await shortcut('o');
  await waitDocument(path.basename(file));
}

// Exercise the real registered IPC and sender guard from main. This does not
// add any renderer-facing path or test API to the production preload.
async function invokeMain(channel, ...args) {
  return application.evaluate(({ipcMain, BrowserWindow}, request) => {
    const contents = BrowserWindow.getAllWindows()[0].webContents;
    return ipcMain._invokeHandlers.get(request.channel)({sender: contents, senderFrame: contents.mainFrame}, ...request.args);
  }, {channel, args});
}

function lockedFixture(file) {
  // A self-authored UI regression fixture with one editable and one locked
  // field match. This fixture is not public-corpus or Hancom qualification.
  const python = process.env.HWP_WORKBENCH_PYTHON || 'python3';
  const script = String.raw`
import re, sys, zipfile
source, output = sys.argv[1:]
with zipfile.ZipFile(source) as original, zipfile.ZipFile(output, 'w') as edited:
    for info in original.infolist():
        data = original.read(info.filename)
        if info.filename == 'Contents/section0.xml':
            counter = [0]
            def text(match):
                counter[0] += 1
                if counter[0] == 1:
                    return b'<hp:t>C2 match editable</hp:t>'
                if counter[0] == 2:
                    return b'<hp:fieldBegin><hp:t>C2 match locked</hp:t></hp:fieldBegin>'
                return match.group(0)
            data = re.sub(rb'<hp:t>.*?</hp:t>', text, data, flags=re.S)
        edited.writestr(info, data)
`;
  const result = spawnSync(python, ['-c', script, path.join(root, 'fixtures/generated/review-form.hwpx'), file], {encoding: 'utf8'});
  assert.equal(result.status, 0, result.stderr);
}

try {
  application = await electron.launch({args: [root, `--user-data-dir=${temporary}`], cwd: root, timeout: 30000});
  await application.evaluate(({dialog, shell}) => {
    dialog.showMessageBox = async () => ({response: 1});
    shell.showItemInFolder = file => { globalThis.testRevealedOutput = file; };
  });
  window = await application.firstWindow();
  const errors = [];
  window.on('pageerror', error => errors.push(error.message));
  window.on('dialog', dialog => dialog.accept());
  await window.waitForSelector('#emptyOpen');
  await window.evaluate(() => {
    localStorage.setItem('hwp-language', 'en');
    localStorage.setItem('hwp-theme', 'light');
  });
  await window.reload();
  await window.waitForSelector('#emptyOpen');
  assert.equal(await window.locator('#empty p').count(), 0);
  assert.equal(await window.locator('#empty .drop').count(), 1);
  assert.equal(await window.locator('#empty .ctas button').count(), 2);
  assert.equal(await window.locator('#save').isVisible(), false);
  assert.equal(await window.locator('#language').count(), 0);
  assert.equal(await window.locator('#recentRow').isVisible(), false);
  assert.equal(await window.evaluate(() => Object.isFrozen(window.hwp)), true);
  await assert.rejects(invokeMain('shell:reveal-output'), /No saved copy/);
  const fontEvidence = await window.evaluate(async () => {
    const {createFontMapper} = await import('./fonts.js');
    const mapper = createFontMapper(() => {});
    const svg = mapper.mapSvg('<svg xmlns="http://www.w3.org/2000/svg"><text font-family="없는 고딕">Basic information</text></svg>');
    const family = new DOMParser().parseFromString(svg, 'image/svg+xml').querySelector('text').getAttribute('font-family');
    const context = document.createElement('canvas').getContext('2d');
    context.font = `16px ${family}`;
    return {family, expected: context.measureText('Basic information').width, actual: mapper.measure('16px "없는 고딕"', 'Basic information')};
  });
  assert.equal(fontEvidence.family, '"Apple SD Gothic Neo", sans-serif');
  assert.equal(fontEvidence.actual, fontEvidence.expected);
  checks.push('Welcome shell, matching canvas/SVG substitutions and reveal rejection before saving');

  await window.locator('#paletteButton').click();
  await window.waitForSelector('#palette[open]');
  await window.keyboard.press('Tab');
  assert.equal(await window.locator('#palette .scope .on').innerText(), 'Commands');
  await window.keyboard.press('Escape');
  assert.equal(await window.evaluate(() => document.activeElement.id), 'paletteButton');
  await command('Sample');
  await waitDocument('review-form.hwpx');
  assert.equal(await window.locator('#workspace p').count(), 0);
  assert.equal(await window.locator('#panel').isVisible(), false);
  const closedWidth = await window.locator('#canvas').evaluate(element => element.clientWidth);
  await shortcut('Alt+i');
  assert.equal(await window.locator('#checksPanel').isVisible(), true);
  assert.ok(await window.locator('#canvas').evaluate(element => element.clientWidth) < closedWidth);
  assert.equal(await window.locator('#warningDetails').getAttribute('open'), null);
  await shortcut('Alt+i');
  assert.equal(await window.locator('#panel').isVisible(), false);
  await shortcut('k');
  await window.locator('#paletteQuery').fill('Basic information');
  await window.keyboard.press('Tab');
  await window.keyboard.press('Tab');
  await window.keyboard.press('Enter');
  await window.waitForSelector('.slip');
  assert.equal(await window.evaluate(() => document.activeElement.id), 'textEdit');
  await window.keyboard.press('Escape');
  checks.push('Palette command and text selection, scopes, focus return and panel recentering');

  for (const theme of ['Dark', 'Light']) {
    await command(`Theme ${theme}`);
    assert.equal(await window.evaluate(() => document.documentElement.dataset.theme), theme.toLowerCase());
    const desk = await window.evaluate(() => getComputedStyle(document.body).backgroundColor);
    assert.equal(desk, theme === 'Dark' ? 'rgb(15, 15, 14)' : 'rgb(232, 231, 227)');
    if (await window.locator('.page').count()) {
      assert.equal(await window.locator('.page').first().evaluate(page => getComputedStyle(page).backgroundColor), 'rgb(255, 255, 255)');
    }
  }
  await shortcut('/');
  assert.ok(await window.locator('#shortcutItems .shortcut').count() >= 15);
  await window.keyboard.press('Escape');
  await window.emulateMedia({reducedMotion: 'reduce'});
  assert.equal(await window.locator('#dropSheet').evaluate(element => getComputedStyle(element).transitionDuration), '0s');
  checks.push('Both theme overrides, white-page marks, shortcut sheet and reduced motion');

  const fixture = path.join(temporary, 'locked-find.hwpx');
  lockedFixture(fixture);
  await openFile(fixture);
  await shortcut('Alt+f');
  await window.locator('#findQuery').fill('C2 match');
  assert.equal(await window.locator('#findCount').innerText(), '1 / 2');
  await window.locator('#replaceQuery').fill('C2 changed');
  await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
  await window.locator('#textEdit').fill('Unstaged draft');
  await window.locator('#replaceAll').click();
  assert.equal(await window.locator('#changes .change').count(), 0);
  assert.equal(await window.locator('#textEdit').inputValue(), 'Unstaged draft');
  await window.locator('#textEdit').focus();
  await window.keyboard.press('Escape');
  await window.locator('#replaceAll').click();
  assert.match(await window.locator('#dirty').innerText(), /1/);
  assert.equal(await window.locator('#changes .change').count(), 1);
  assert.match(await window.locator('#skippedPopover').innerText(), /Locked/);
  await window.keyboard.press('Escape'); // skip popover
  await window.locator('#findClose').click();
  await window.locator('#toast button').click();
  assert.equal(await window.locator('#changes .change').count(), 0);
  await shortcut('Shift+z');
  assert.equal(await window.locator('#changes .change').count(), 1);
  await window.locator('#save').click();
  await window.waitForSelector('#reviewDialog[open]');
  assert.equal(await window.locator('#confirmSave').isDisabled(), true);
  await window.locator('#acknowledge').check();
  await window.locator('#reviewChanges .acts button').click();
  assert.equal(await window.locator('#acknowledge').isChecked(), false);
  assert.equal(await window.locator('#confirmSave').isDisabled(), true);
  await window.locator('#cancelReview').click();
  checks.push('Synthetic locked-field find/replace, skip reason, toast Undo and review removal');

  // Qualify real page interactions separately when the public sample exists.
  const publicFile = path.join(root, 'fixtures/public/hwp-cli-report-tables.hwpx');
  let source = publicFile;
  try {
    await fs.access(publicFile);
  } catch {
    skipped.push('Public HWPX page/edit/save test: fixture not downloaded');
    source = path.join(root, 'fixtures/generated/review-form.hwpx');
  }
  await openFile(source);
  if (source === publicFile) {
    await window.waitForSelector('.page-image');
    const thumbnailCount = await window.locator('#thumbnails .sheet').count();
    assert.ok(thumbnailCount > 0);
    await window.waitForFunction(count => {
      const images = [...document.querySelectorAll('#thumbnails .sheet img')];
      return images.length === count && images.every(image => image.complete && image.naturalWidth > 0);
    }, thumbnailCount);
    assert.equal(await window.locator('#thumbnails .sheet img').count(), thumbnailCount);
  }
  const hit = window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first();
  await hit.click();
  await window.waitForSelector('.slip');
  const original = await window.locator('#textEdit').inputValue();
  assert.equal(await window.locator('#textEdit').evaluate(element => getComputedStyle(element).outlineStyle), 'none');
  await window.locator('#textEdit').fill(original + ' 검토');
  await window.locator('#applyText').click();
  assert.match(await window.locator('#dirty').innerText(), /1/);
  assert.equal(await window.locator('#panel').isVisible(), true);
  assert.match(await window.locator('#changes ins').innerText(), /검토/);
  await application.evaluate(({Menu}) => {
    const item = Menu.getApplicationMenu().getMenuItemById('undo');
    item.click();
  });
  await window.waitForFunction(() => document.querySelectorAll('#changes .change').length === 0);
  assert.equal(await window.locator('#changes .change').count(), 0);
  await shortcut('Shift+z');
  assert.equal(await window.locator('#changes .change').count(), 1);
  const before = await fs.readFile(source);
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
  assert.match(await window.locator('#toast').innerText(), /Saved review-copy.hwpx/);
  await window.locator('#toast button').click();
  assert.equal(await application.evaluate(() => globalThis.testRevealedOutput), output);
  await assert.rejects(invokeMain('shell:reveal-output', '/tmp/other.hwpx'), /arguments/);
  assert.deepEqual(await fs.readFile(source), before);
  const savedBytes = await fs.readFile(output);
  await window.locator('#save').click();
  // The closed dialog keeps the earlier tick until openReview resets it, and
  // check() returns at once for a ticked box, so wait for the open dialog.
  await window.waitForSelector('#reviewDialog[open]');
  await window.locator('#acknowledge').check();
  await window.locator('#confirmSave').click();
  await window.waitForFunction(() => document.getElementById('status').classList.contains('error'));
  assert.deepEqual(await fs.readFile(output), savedBytes);
  await window.locator('#closeReview').click();
  await window.locator('#changes .acts button').click();
  checks.push(`${source === publicFile ? 'Public HWPX' : 'Synthetic sample'}: slip, progressive rail, native save, reveal and no overwrite`);

  const recent = await window.evaluate(() => window.hwp.recent());
  assert.ok(recent.length > 0 && recent.length <= 8);
  assert.ok(recent.every(item => !('path' in item) && !('file' in item) && typeof item.id === 'string'));
  if (await window.locator('.page-image').count()) {
    assert.match(recent[0].thumbnail, /^data:image\/png;base64,/);
  }
  const stored = JSON.parse(await fs.readFile(path.join(temporary, 'recent.json'), 'utf8'));
  assert.equal(stored[0].id, recent[0].id);
  await assert.rejects(invokeMain('document:open-recent', 0), /Invalid recent/);
  await assert.rejects(invokeMain('document:open-dropped', path.join(temporary, 'reject.txt')), /Unsupported/);
  await command('Language');
  const language = await window.evaluate(() => document.documentElement.lang);
  assert.equal(language, 'ko');
  await window.reload();
  await window.waitForSelector('#recentItems .doc-card');
  assert.equal(await window.evaluate(() => document.documentElement.lang), language);
  await window.locator('#recentItems .doc-card').first().click();
  await waitDocument(path.basename(source));
  checks.push('Recent list persistence and reopen by id, PNG cache, drop extension rejection and Korean persistence');

  for (const url of ['app://workbench/../package.json', 'app://workbench/unknown.xyz']) {
    const response = await serveAppRequest({url, method: 'GET'}, {
      desktop: path.join(root, 'desktop'), vendor: path.join(root, 'node_modules/@rhwp/core')
    });
    assert.notEqual(response.status, 200, url);
  }
  assert.notEqual(await window.evaluate(async () => (await fetch('app://workbench/unknown.xyz')).status), 200);
  assert.deepEqual(errors, []);
  checks.push('Protocol rejects traversal and unknown types; no renderer errors');
  console.log(JSON.stringify({host: `${process.platform}/${process.arch}`, checks, skipped, notTested: ['Hancom reopen/layout', 'Real Korean IME', 'Signing/packaging']}, null, 2));
} finally {
  if (application) {
    await application.evaluate(({app, dialog}) => {
      dialog.showMessageBox = async () => ({response: 1});
      app.exit(0);
    }).catch(() => {});
    await application.close().catch(() => {});
  }
  await fs.rm(temporary, {recursive: true, force: true});
}
