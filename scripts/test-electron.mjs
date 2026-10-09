import {_electron as electron} from 'playwright';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createRequire} from 'node:module';
import {spawn, spawnSync} from 'node:child_process';

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

function engineRequest(directory, method, args) {
  const python = process.env.HWP_WORKBENCH_PYTHON || 'python3';
  const result = spawnSync(python, ['-m', 'hwp_workbench', 'rpc', '--root', directory], {
    cwd: root, shell: false, encoding: 'utf8',
    env: {...process.env, PYTHONPATH: path.join(root, 'engine'), PYTHONUTF8: '1'},
    input: JSON.stringify({method, args})
  });
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(result.stdout);
}

async function waitForMain(predicate, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (!(await application.evaluate(predicate))) {
    assert.ok(Date.now() < deadline, 'Timed out waiting for the main process');
    await new Promise(resolve => setTimeout(resolve, 100));
  }
}

// Main writes agent files after the click handler returns.
async function waitForFile(file, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (true) {
    try {
      return await fs.readFile(file, 'utf8');
    } catch (error) {
      if (error.code !== 'ENOENT' || Date.now() > deadline) {
        throw error;
      }
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
}

async function waitPreviewText(text) {
  await window.waitForFunction(async text => {
    for (const image of document.querySelectorAll('.page-image')) {
      let svg;
      try {
        svg = await (await fetch(image.src)).text();
      } catch {
        // A newer preview can revoke this page while the assertion polls.
        continue;
      }
      const layout = new DOMParser().parseFromString(svg, 'image/svg+xml');
      const renderedText = [...layout.querySelectorAll('text')].map(node => node.textContent).join('');
      if (renderedText.includes(text)) {
        return true;
      }
    }
    return false;
  }, text);
}

async function secondLaunch(file) {
  const child = spawn(require('electron'), [root, `--user-data-dir=${temporary}`, '--open', file], {
    cwd: root, shell: false, stdio: 'ignore'
  });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      reject(Error('Secondary Electron did not exit'));
    }, 15000);
    child.on('error', error => {
      clearTimeout(timer);
      reject(error);
    });
    child.on('exit', code => {
      clearTimeout(timer);
      code === 0 ? resolve() : reject(Error(`Secondary Electron exit ${code}`));
    });
  });
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
  // The window exists only after firstWindow() resolves.
  await application.evaluate(({BrowserWindow}) => BrowserWindow.getAllWindows()[0].setSize(1440, 900));
  const errors = [];
  window.on('pageerror', error => errors.push(error.message));
  // A dialog can close before the handler runs (for example on a document switch).
  window.on('dialog', dialog => dialog.accept().catch(() => {}));
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
  // C4: daily Windows Hangul keys run with physical codes in the default scheme.
  await window.keyboard.press('Control+f');
  await window.waitForSelector('#findBar:not([hidden])');
  await window.locator('#findClose').click();
  await application.evaluate(({dialog}, file) => {
    dialog.showOpenDialog = async () => ({canceled: false, filePaths: [file]});
  }, path.join(root, 'fixtures/generated/review-form.hwpx'));
  await window.keyboard.press('Alt+o');
  await waitDocument('review-form.hwpx');
  await window.keyboard.press('Control+g');
  assert.match(await window.locator('#chordPending').innerText(), /G/);
  await window.keyboard.press('c');
  assert.equal(await window.locator('body').evaluate(body => body.classList.contains('control-marks')), true);
  await window.waitForSelector('.control-mark');
  await window.keyboard.press('Control+g');
  await window.keyboard.press('c');
  await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
  await window.keyboard.press('Alt+l');
  await window.waitForSelector('#formatDialog[open]');
  assert.match(await window.locator('#formatDialog').innerText(), /Character shape/);
  assert.match(await window.locator('#formatDialog').innerText(), /Apple SD Gothic Neo/);
  assert.match(await window.locator('#formatDialog').innerText(), /22 pt/);
  assert.equal(await window.locator('#formatDialog').evaluate(dialog => dialog.getBoundingClientRect().width), 360);
  await window.waitForFunction(() => {
    const card = document.getElementById('formatDialog').getBoundingClientRect();
    const slip = document.querySelector('.slip').getBoundingClientRect();
    return card.left >= slip.right + 11 || card.right <= slip.left - 11 ||
      card.top >= slip.bottom + 11 || card.bottom <= slip.top - 11;
  });
  await window.keyboard.press('Escape');
  assert.equal(await window.evaluate(() => document.activeElement.id), 'textEdit');
  const symbolOriginal = await window.locator('#textEdit').inputValue();
  await window.locator('#textEdit').evaluate(editor => editor.setSelectionRange(0, 0));
  await window.keyboard.press('Control+F10');
  await window.waitForSelector('#symbolDialog[open]');
  assert.equal(await window.evaluate(() => document.activeElement.id), 'symbolQuery');
  const symbolFocus = await window.locator('#symbolQuery').evaluate(input => {
    const style = getComputedStyle(input);
    return {outline: style.outlineStyle, shadow: style.boxShadow, caret: style.caretColor, ink: style.color};
  });
  assert.equal(symbolFocus.outline, 'none');
  assert.equal(symbolFocus.shadow, 'none');
  assert.equal(symbolFocus.caret, symbolFocus.ink);
  await window.locator('#symbolQuery').fill('reference');
  await window.locator('.symbol-choice').first().click();
  assert.equal(await window.locator('#textEdit').inputValue(), '※' + symbolOriginal);
  await window.locator('#textEdit').fill(symbolOriginal);
  await window.keyboard.press('Escape');
  await window.keyboard.press('Alt+l');
  await window.waitForSelector('#formatDialog[open]');
  await window.waitForFunction(() => {
    const card = document.getElementById('formatDialog').getBoundingClientRect();
    return Math.abs(card.left - (innerWidth - card.width) / 2) < 1 &&
      Math.abs(card.top - (innerHeight - card.height) / 2) < 1;
  });
  await window.keyboard.press('Escape');
  const cellDocument = engineRequest(root, 'inspect', {path: 'fixtures/generated/review-form.hwpx'});
  const cellNodes = cellDocument.nodes.filter(node => node.role === 'table-cell' && node.editable);
  await window.locator(`[data-node-id="${cellNodes[0].id}"]`).first().focus();
  await window.keyboard.press('F5');
  await window.waitForSelector('.slip');
  await window.locator('#textEdit').fill(cellNodes[0].text + ' C4');
  await window.keyboard.press('Tab');
  await window.waitForFunction(text => document.getElementById('textEdit')?.value === text, cellNodes[1].text);
  await window.keyboard.press('Shift+Tab');
  await window.waitForFunction(text => document.getElementById('textEdit')?.value === text, cellNodes[0].text + ' C4');
  await window.waitForFunction(() => {
    const canvas = document.getElementById('canvas');
    return canvas.scrollWidth <= canvas.clientWidth + 1;
  });
  const canvasFit = await window.locator('#canvas').evaluate(canvas => canvas.scrollWidth <= canvas.clientWidth + 1);
  assert.equal(canvasFit, true);
  const railFit = await window.locator('#thumbnails .sheet').first().evaluate(sheet => {
    const bounds = sheet.getBoundingClientRect();
    const rail = sheet.closest('.rail').getBoundingClientRect();
    return bounds.width === 56 && bounds.height === 79 && bounds.left >= rail.left && bounds.right <= rail.right;
  });
  assert.equal(railFit, true);
  await window.keyboard.press('Escape');
  await shortcut('z');
  await window.waitForFunction(() => document.querySelectorAll('#changes .change').length === 0);
  checks.push('Windows Control-F, Option-O, Control-G C, Option-L values, F5 and cell Tab, Control-F10 symbol insertion, panel/rail fit');

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

  // Replays the key sequence that a real macOS 2-Set Korean run produced:
  // Enter commits a syllable (isComposing) and then arrives again as a plain
  // Enter; ⌘↵ during composition arrives once, with isComposing set.
  await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
  await window.waitForSelector('.slip');
  const imeKeys = await window.evaluate(async () => {
    const editor = document.getElementById('textEdit');
    const press = (key, init = {}) => {
      const event = new KeyboardEvent('keydown', {key, bubbles: true, cancelable: true, ...init});
      editor.dispatchEvent(event);
      return event.defaultPrevented;
    };
    const composingEnterPassedToInput = !press('Enter', {isComposing: true});
    const plainEnterBlocked = press('Enter');
    const optionReturnPassedToInput = !press('Enter', {altKey: true});
    press('Escape', {isComposing: true});
    const openAfterComposingEscape = !!document.querySelector('.slip');
    editor.value += ' 한글';
    editor.dispatchEvent(new Event('input'));
    press('Enter', {metaKey: true, isComposing: true});
    const stagedBeforeCommit = document.querySelectorAll('#changes .change').length;
    editor.dispatchEvent(new CompositionEvent('compositionend', {data: '글'}));
    await new Promise(resolve => setTimeout(resolve, 50));
    return {
      composingEnterPassedToInput, plainEnterBlocked, optionReturnPassedToInput, openAfterComposingEscape, stagedBeforeCommit,
      staged: document.querySelectorAll('#changes .change').length,
      inserted: document.querySelector('#changes ins')?.textContent,
      slipOpen: !!document.querySelector('.slip')
    };
  });
  assert.deepEqual(imeKeys, {
    composingEnterPassedToInput: true, plainEnterBlocked: true, optionReturnPassedToInput: true, openAfterComposingEscape: true,
    stagedBeforeCommit: 0, staged: 1, inserted: ' 한글', slipOpen: false
  });
  await shortcut('z');
  await window.waitForFunction(() => document.querySelectorAll('#changes .change').length === 0);
  checks.push('Korean IME keys: Enter adds no line break, ⌘↵ during composition stages once after commit');

  // CDP lets Chromium commit actual composition text into the focused control.
  // The real OS-key sequence above remains a separate regression replay.
  await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
  const cdpOriginal = await window.locator('#textEdit').inputValue();
  await window.locator('#textEdit').focus();
  await window.locator('#textEdit').evaluate(editor => {
    editor.setSelectionRange(editor.value.length, editor.value.length);
  });
  const cdp = await window.context().newCDPSession(window);
  await cdp.send('Input.insertText', {text: ' '});
  await cdp.send('Input.imeSetComposition', {text: '한글', selectionStart: 2, selectionEnd: 2});
  await cdp.send('Input.insertText', {text: '한글'});
  assert.equal(await window.locator('#textEdit').inputValue(), cdpOriginal + ' 한글');
  await window.locator('#applyText').click();
  assert.equal(await window.locator('#changes .change').count(), 1);
  assert.equal(await window.locator('#changes ins').innerText(), ' 한글');
  await cdp.detach();
  await waitPreviewText('한글');
  await shortcut('z');
  await window.waitForFunction(() => document.querySelectorAll('#changes .change').length === 0);
  checks.push('CDP Korean composition commits once; memory preview renders the composed text');


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
  assert.equal(await window.locator('#keyScheme').getAttribute('role'), 'radiogroup');
  assert.equal(await window.locator('#keyScheme select').count(), 0);
  const windowsScheme = window.locator('#keyScheme [data-scheme="windows"]');
  const macScheme = window.locator('#keyScheme [data-scheme="mac"]');
  await windowsScheme.focus();
  await window.keyboard.press('ArrowRight');
  assert.equal(await macScheme.getAttribute('aria-checked'), 'true');
  assert.equal(await macScheme.getAttribute('tabindex'), '0');
  assert.equal(await window.evaluate(() => document.activeElement.dataset.scheme), 'mac');
  assert.equal(await window.evaluate(() => localStorage.getItem('hwp-key-scheme')), 'mac');
  await window.keyboard.press('ArrowRight');
  assert.equal(await windowsScheme.getAttribute('aria-checked'), 'true');
  assert.equal(await window.evaluate(() => document.activeElement.dataset.scheme), 'windows');
  assert.equal(await window.locator('#shortcutsDialog').evaluate(dialog => dialog.open), true);
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
  assert.equal(await window.locator('#panel').isVisible(), true);
  await window.waitForFunction(() => {
    const canvas = document.getElementById('canvas');
    return canvas.scrollWidth <= canvas.clientWidth + 1;
  });
  assert.match(await window.locator('#skippedPopover').innerText(), /Locked/);
  await window.keyboard.press('Escape'); // skip popover
  await window.locator('#findClose').click();
  await window.locator('#toast button').click();
  assert.equal(await window.locator('#changes .change').count(), 0);
  await shortcut('Shift+z');
  assert.equal(await window.locator('#changes .change').count(), 1);
  await window.locator('#save').click();
  await window.waitForSelector('#reviewDialog[open]');
  assert.equal(await window.evaluate(() => document.activeElement.id), 'reviewChanges');
  await window.keyboard.press('Shift+Tab');
  assert.equal(await window.evaluate(() => document.activeElement.id), 'closeReview');
  await window.keyboard.press('Tab');
  assert.equal(await window.evaluate(() => document.activeElement.id), 'reviewChanges');
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
  await shortcut('Shift+s');
  await window.waitForSelector('#reviewDialog[open]');
  assert.equal(await window.locator('#confirmSave').isDisabled(), true);
  await window.locator('#acknowledge').check();
  await window.locator('#confirmSave').click();
  await window.waitForFunction(() => !document.getElementById('reviewDialog').open);
  assert.ok((await fs.stat(output)).size > 0);
  assert.match(await window.locator('#toast').innerText(), /Saved review-copy.hwpx/);
  await window.locator('#toast button').click();
  // Main runs IPC requests in order, so the reveal can wait behind a preview render.
  await waitForMain(() => globalThis.testRevealedOutput);
  assert.equal(await application.evaluate(() => globalThis.testRevealedOutput), output);
  await assert.rejects(invokeMain('shell:reveal-output', '/tmp/other.hwpx'), /arguments/);
  assert.deepEqual(await fs.readFile(source), before);
  const savedBytes = await fs.readFile(output);
  await shortcut('Shift+s');
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

  const inPlace = path.join(temporary, 'in-place.hwpx');
  await fs.copyFile(source, inPlace);
  await openFile(inPlace);
  const baseBytes = await fs.readFile(inPlace);
  for (let saveIndex = 0; saveIndex < 2; saveIndex += 1) {
    await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
    const value = await window.locator('#textEdit').inputValue();
    await window.locator('#textEdit').fill(value + ' 저장');
    await window.locator('#applyText').click();
    if (saveIndex === 0) {
      await command('Save');
    } else {
      await shortcut('s');
    }
    if (saveIndex === 0) {
      await window.waitForSelector('#reviewDialog[open]');
      await window.locator('#acknowledge').check();
      await window.locator('#confirmSave').click();
    }
    await window.waitForFunction(() => document.getElementById('dirty').hidden && document.getElementById('toast').textContent.includes('Saved'));
    assert.equal(await window.locator('#reviewDialog').evaluate(item => item.open), false);
  }
  assert.notDeepEqual(await fs.readFile(inPlace), baseBytes);
  assert.equal((await fs.readdir(path.join(temporary, '.hwp-workbench/backups'))).filter(name => name.startsWith('in-place.')).length, 2);
  await window.locator('#toast button').click();
  await waitForMain(() => globalThis.testRevealedOutput?.includes('/.hwp-workbench/backups/'));
  await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
  await window.locator('#textEdit').fill('conflict');
  await window.locator('#applyText').click();
  await fs.writeFile(inPlace, baseBytes);
  await application.evaluate(({dialog}) => {
    dialog.showMessageBox = async (_win, options) => {
      globalThis.testConflictDialog = options;
      return {response: 2};
    };
  });
  await shortcut('s');
  await waitForMain(() => globalThis.testConflictDialog?.buttons.length === 3);
  assert.deepEqual(await fs.readFile(inPlace), baseBytes);
  await window.locator('#changes .acts button').click();
  checks.push('In-place save twice, two kept versions, direct later save, conflict dialog refuses replacement');

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
  // The newest recent is whichever document the steps above opened last.
  await waitDocument(recent[0].name);
  checks.push('Recent list persistence and reopen by id, PNG cache, drop extension rejection and Korean persistence');

  await command('Language');
  const inboxSource = path.join(temporary, 'agent-review.hwpx');
  await fs.copyFile(path.join(root, 'fixtures/generated/review-form.hwpx'), inboxSource);
  await openFile(inboxSource);
  await command('Agent');
  await window.locator('#agentLinkSwitch').check();
  const inspected = engineRequest(temporary, 'inspect', {path: path.basename(inboxSource)});
  const proposalNodes = inspected.nodes.filter(node => node.editable).slice(0, 2);
  assert.equal(proposalNodes.length, 2);
  const proposedPatch = {schemaVersion: 1, baseSha256: inspected.sha256, operations: proposalNodes.map((node, index) => ({
    op: 'set_text', nodeId: node.id, expectedText: node.text, text: node.text + ` C3 agent ${index}`
  }))};
  const proposed = engineRequest(temporary, 'propose', {path: path.basename(inboxSource), patch: proposedPatch,
    title: 'C3 <review>', author: 'Claude'});
  const proposalCard = window.locator('#agentInbox .proposal').first();
  await proposalCard.waitFor();
  assert.equal(await proposalCard.locator('.proposal-title').innerText(), 'C3 <review>');
  assert.equal(await proposalCard.locator('.steps span').count(), 3);
  assert.match(await window.locator('#toast').innerText(), /Claude proposed 2 edits/);
  await proposalCard.locator('.proposal-change').first().locator('.acts button').first().click();
  await window.waitForFunction(() => document.querySelectorAll('#changes .change.agent').length === 1);
  assert.match(await window.locator('#changes .who').innerText(), /Claude/);
  await waitPreviewText('C3 agent 0');
  // Marks are drawn after the preview page image loads and its hit map is
  // rebuilt, which is later than the SVG text becoming available.
  await window.waitForSelector('.mark-line.agent.range');
  await window.waitForSelector('.margin-mark.agent');
  await proposalCard.locator('.proposal-change').nth(1).locator('.acts button').nth(1).click();
  const outcomePath = path.join(temporary, proposed.proposal.replace(/\.json$/, '.outcome.json'));
  const outcome = JSON.parse(await waitForFile(outcomePath));
  assert.equal(outcome.status, 'partial');
  assert.deepEqual(outcome.acceptedNodeIds, [proposalNodes[0].id]);
  assert.deepEqual(outcome.declinedNodeIds, [proposalNodes[1].id]);
  assert.equal(await window.locator('#changes .change').count(), 1);
  await window.locator('#agentLinkSwitch').uncheck();
  checks.push('Watched proposal, blue partial apply, rendered text and exclusive outcome node IDs');

  const handoffSource = path.join(temporary, 'handoff.hwpx');
  await fs.copyFile(path.join(root, 'fixtures/generated/review-form.hwpx'), handoffSource);
  await application.evaluate(({dialog}) => {
    globalThis.testHandoffQuestions = 0;
    dialog.showMessageBox = async () => {
      globalThis.testHandoffQuestions += 1;
      return {response: 0};
    };
  });
  await secondLaunch(handoffSource);
  await application.evaluate(async () => {
    const deadline = Date.now() + 10000;
    while (!globalThis.testHandoffQuestions && Date.now() < deadline) {
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    if (!globalThis.testHandoffQuestions) {
      throw Error('Native handoff question missing');
    }
  });
  assert.equal(await window.locator('#filename').innerText(), path.basename(inboxSource));
  assert.equal(await window.locator('#changes .change').count(), 1);
  await application.evaluate(({dialog}) => { dialog.showMessageBox = async () => ({response: 1}); });
  await secondLaunch(handoffSource);
  await waitDocument(path.basename(handoffSource));
  assert.equal(await window.locator('#changes .change').count(), 0);
  checks.push('Second launch --open preserves staged edits on cancel and switches after native approval');


  for (const url of ['app://workbench/../package.json', 'app://workbench/unknown.xyz']) {
    const response = await serveAppRequest({url, method: 'GET'}, {
      desktop: path.join(root, 'desktop'), vendor: path.join(root, 'node_modules/@rhwp/core')
    });
    assert.notEqual(response.status, 200, url);
  }
  assert.notEqual(await window.evaluate(async () => (await fetch('app://workbench/unknown.xyz')).status), 200);
  assert.deepEqual(errors, []);
  checks.push('Protocol rejects traversal and unknown types; no renderer errors');
  console.log(JSON.stringify({host: `${process.platform}/${process.arch}`, checks, skipped, notTested: ['Hancom reopen/layout', 'Real Korean IME (manual: scripts/qa/real-ime.mjs)', 'Signing/packaging']}, null, 2));
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
