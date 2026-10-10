import {webkit} from 'playwright';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawn, spawnSync} from 'node:child_process';
import {createInterface} from 'node:readline';
import {buildFrontend} from './build-frontend.mjs';
import {cargoCommand, rustRpc} from './rust-cli.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const cargoWrapper = cargoCommand;
const modifier = process.platform === 'darwin' ? 'Meta' : 'Control';
const checks = [];
const skipped = [];
const errors = [];
const pageConsole = [];
const hostLog = [];
const notTested = ['Native WKWebView/Tauri shell', 'OS menus, real dialogs, clipboard and Finder',
  'OS file-open and second-instance delivery (backend handoff is tested)', 'Chromium CDP IME input (unavailable in WebKit)',
  'Real Korean IME (manual: npm run qa:native)', 'Hancom reopen/layout', 'Signing/packaging'];
let temporary;
let profile;
let browser;
let host;
let hostInfo;
let window;
let currentWorkflow;

function run(command, args) {
  const result = spawnSync(command, args, {cwd: root, stdio: ['ignore', 'inherit', 'inherit'], shell: false});
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `${command} failed`);
}
async function startHost() {
  // Cargo metadata supplies the target directory, including a configured shared target path.
  run(cargoWrapper, ['build', '--locked', '-p', 'app-testhost']);
  const metadata = spawnSync(cargoWrapper, ['metadata', '--no-deps', '--format-version', '1'], {cwd: root, encoding: 'utf8', shell: false});
  assert.equal(metadata.status, 0, metadata.stderr);
  const binary = path.join(JSON.parse(metadata.stdout).target_directory, 'debug', process.platform === 'win32' ? 'app-testhost.exe' : 'app-testhost');
  host = spawn(binary, [root, profile], {cwd: root, stdio: ['ignore', 'pipe', 'pipe'], shell: false});
  const lines = createInterface({input: host.stdout});
  host.stderr.on('data', bytes => {
    hostLog.push(bytes.toString());
    if (hostLog.length > 1000) hostLog.shift();
  });
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(Error('Test host startup timed out')), 30000);
    host.once('error', error => {clearTimeout(timer); reject(error);});
    host.once('exit', (code, signal) => {clearTimeout(timer); reject(Error(`Test host exited: ${code ?? signal}`));});
    lines.once('line', line => {
      clearTimeout(timer);
      try {
        const value = JSON.parse(line);
        assert.equal(new URL(value.url).hostname, '127.0.0.1');
        assert.match(value.token, /^[a-f0-9]{64}$/);
        resolve(value);
      } catch (error) { reject(error); }
    });
    lines.on('line', line => {
      if (!line.includes('"token"')) hostLog.push(line);
    });
  });
}
async function request(route, args = {}) {
  const response = await fetch(new URL(route, hostInfo.url), {
    method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(args),
    signal: AbortSignal.timeout(45000)
  });
  const value = await response.json();
  if (!response.ok) throw Error(value.message ?? String(value));
  return value;
}
const testControl = (action, args) => request(`test/${action}`, args);
const invoke = (command, args) => request(`invoke/${command}`, args);
async function waitHost(predicate, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (!predicate(await testControl('state'))) {
    assert.ok(Date.now() < deadline, 'Timed out waiting for the Rust backend');
    await new Promise(resolve => setTimeout(resolve, 100));
  }
}
async function openFile(file) {
  await testControl('dialog/open', {path: file});
  await shortcut('o');
  await waitDocument(path.basename(file));
}
async function sample() {
  await window.evaluate(() => {localStorage.setItem('hwp-language', 'en'); localStorage.setItem('hwp-theme', 'light'); localStorage.setItem('hwp-key-scheme', 'windows');});
  await window.reload();
  await window.waitForSelector('#emptyOpen');
  await command('Sample');
  await waitDocument('review-form.hwpx');
}
async function saveSource() {
  const publicFile = path.join(root, 'fixtures/public/hwp-cli-report-tables.hwpx');
  try {await fs.access(publicFile); return {source: publicFile, publicFile};}
  catch {
    const reason = 'Public HWPX page/edit/save test: fixture not downloaded';
    if (!skipped.includes(reason)) skipped.push(reason);
    return {source: path.join(root, 'fixtures/generated/review-form.hwpx'), publicFile};
  }
}
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

function engineRequest(directory, method, args) {
  return rustRpc(directory, method, args);
}

// The backend writes agent files after the click handler returns.
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

function lockedFixture(file) {
  // A self-authored UI regression fixture with one editable and one locked
  // field match. This fixture is not public-corpus or Hancom qualification.
  run(cargoWrapper, ['run', '--locked', '--quiet', '-p', 'hwp-core', '--example', 'locked-fixture', '--', file]);
}

const workflows = {
  'welcome': async () => {
    assert.equal(await window.locator('#empty p').count(), 0);
    assert.equal(await window.locator('#empty .drop').count(), 1);
    assert.equal(await window.locator('#empty .ctas button').count(), 2);
    assert.equal(await window.locator('#save').isVisible(), false);
    assert.equal(await window.locator('#language').count(), 0);
    assert.equal(await window.locator('#recentRow').isVisible(), false);
    assert.equal(await window.evaluate(() => Object.isFrozen(window.hwp)), true);
    await assert.rejects(invoke('shell_reveal_output'), /No saved copy/);
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
  },
  'windows-keys': async () => {
    await sample();

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
    await testControl('dialog/open', {path: path.join(root, 'fixtures/generated/review-form.hwpx')});
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
    // Existing Rust fixture: the line-break element is editable as a reserved
    // marker. Test real WebKit caret movement without replacing textarea text.
    await openFile(path.join(root, 'tests/data/fixtures-make_hwpx-f92a949cb17d61c8.hwpx'));
    await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
    const marked = window.locator('#textEdit');
    assert.equal(await marked.inputValue(), 'before\ufdd0after');
    assert.equal(await window.locator('.marker-mirror .inline-marker').count(), 1);
    await fs.mkdir(path.join(root, 'work/qa/webkit'), {recursive: true});
    await window.locator('.slip').screenshot({path: path.join(root, 'work/qa/webkit/inline-marker-slip.png')});
    await marked.evaluate(editor => editor.setSelectionRange(6, 6));
    await window.keyboard.press('ArrowRight');
    assert.equal(await marked.evaluate(editor => editor.selectionStart), 7);
    await window.keyboard.press('ArrowLeft');
    assert.equal(await marked.evaluate(editor => editor.selectionStart), 6);
    await marked.evaluate(editor => {
      editor.dispatchEvent(new CompositionEvent('compositionstart', {bubbles: true}));
      editor.setRangeText('한글', editor.selectionStart, editor.selectionEnd, 'end');
      editor.dispatchEvent(new InputEvent('input', {data: '한글', inputType: 'insertCompositionText', isComposing: true, bubbles: true}));
      editor.dispatchEvent(new CompositionEvent('compositionend', {data: '한글', bubbles: true}));
    });
    assert.equal(await marked.inputValue(), 'before한글\ufdd0after');
    await window.keyboard.press('ArrowRight');
    await window.keyboard.press('Backspace');
    assert.equal(await marked.inputValue(), 'before한글after');
    assert.equal(await window.locator('.marker-mirror .inline-marker').count(), 0);
    await window.locator('#applyText').click();
    assert.equal(await window.locator('#changes .change').count(), 1);
  },
  'palette': async () => {
    await sample();

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
  },
  'ime-keys': async () => {
    await sample();

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

    for (const stage of [false, true]) {
      await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
      const webkitKeys = await window.locator('#textEdit').evaluate(async (editor, stage) => {
        const original = editor.value;
        editor.dispatchEvent(new CompositionEvent('compositionstart', {bubbles: true}));
        editor.value += ' 한글';
        editor.dispatchEvent(new InputEvent('input', {bubbles: true, data: '글', isComposing: true}));
        editor.dispatchEvent(new CompositionEvent('compositionend', {bubbles: true, data: '글'}));
        const enter = new KeyboardEvent('keydown', {key: 'Enter', code: 'Enter', keyCode: 229,
          metaKey: stage, isComposing: false, bubbles: true, cancelable: true});
        editor.dispatchEvent(enter);
        const stagesBeforeTimer = document.querySelectorAll('#changes .change').length;
        const plain = new KeyboardEvent('keydown', {key: 'Enter', bubbles: true, cancelable: true});
        if (!stage) editor.dispatchEvent(plain);
        await new Promise(resolve => setTimeout(resolve, 50));
        return {keyCode: enter.keyCode, stagesBeforeTimer, stages: document.querySelectorAll('#changes .change').length,
          plainBlocked: stage || plain.defaultPrevented, value: editor.value, original,
          inserted: document.querySelector('#changes ins')?.textContent ?? null};
      }, stage);
      assert.equal(webkitKeys.keyCode, 229);
      assert.equal(webkitKeys.stagesBeforeTimer, 0);
      assert.equal(webkitKeys.stages, stage ? 1 : 0);
      assert.equal(webkitKeys.plainBlocked, true);
      assert.equal(webkitKeys.value, webkitKeys.original + ' 한글');
      assert.equal(webkitKeys.inserted, stage ? ' 한글' : null);
      if (stage) {
        await shortcut('z');
        await window.waitForFunction(() => document.querySelectorAll('#changes .change').length === 0);
      } else {
        await window.keyboard.press('Escape');
      }
    }
  },
  'composition-preview': async () => {
    await sample();

    // WebKit has no CDP Input domain. Replay composition and retain the edit/preview assertions.
    await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
    const compositionOriginal = await window.locator('#textEdit').inputValue();
    await window.locator('#textEdit').focus();
    await window.locator('#textEdit').evaluate(editor => {
      editor.setSelectionRange(editor.value.length, editor.value.length);
      editor.dispatchEvent(new CompositionEvent('compositionstart', {bubbles: true}));
      editor.value += ' 한글';
      editor.dispatchEvent(new CompositionEvent('compositionupdate', {data: '한글', bubbles: true}));
      editor.dispatchEvent(new InputEvent('input', {data: '한글', inputType: 'insertCompositionText', isComposing: true, bubbles: true}));
      editor.dispatchEvent(new CompositionEvent('compositionend', {data: '한글', bubbles: true}));
    });
    assert.equal(await window.locator('#textEdit').inputValue(), compositionOriginal + ' 한글');
    await window.locator('#applyText').click();
    assert.equal(await window.locator('#changes .change').count(), 1);
    assert.equal(await window.locator('#changes ins').innerText(), ' 한글');
    await waitPreviewText('한글');
    await shortcut('z');
    await window.waitForFunction(() => document.querySelectorAll('#changes .change').length === 0);
  },
  'themes-shortcuts': async () => {
    await sample();
    let nativePage;
    for (const theme of ['Dark', 'Light']) {
      await command(`Theme ${theme}`);
      assert.equal(await window.evaluate(() => document.documentElement.dataset.theme), theme.toLowerCase());
      const desk = await window.evaluate(() => getComputedStyle(document.body).backgroundColor);
      assert.equal(desk, theme === 'Dark' ? 'rgb(15, 15, 14)' : 'rgb(232, 231, 227)');
      if (await window.locator('.page').count()) {
        assert.equal(await window.locator('.page').first().evaluate(page => getComputedStyle(page).backgroundColor), 'rgb(255, 255, 255)');
      }
      const pageImage = await window.locator('#pages .page-image').first().evaluate(image => {
        const canvas = document.createElement('canvas');
        canvas.width = image.naturalWidth;
        canvas.height = image.naturalHeight;
        canvas.getContext('2d').drawImage(image, 0, 0);
        return canvas.toDataURL('image/png');
      });
      if (nativePage) {
        assert.equal(pageImage === nativePage, true, 'Native document pixels must stay the same in both themes');
      }
      nativePage = pageImage;
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
  },
  'locked-replace': async () => {

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
  },
  'save-copy': async () => {

    const {source, publicFile} = await saveSource();
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
    await testControl('menu', {command: 'undo'});
    await window.waitForFunction(() => document.querySelectorAll('#changes .change').length === 0);
    assert.equal(await window.locator('#changes .change').count(), 0);
    await shortcut('Shift+z');
    assert.equal(await window.locator('#changes .change').count(), 1);
    const before = await fs.readFile(source);
    const output = path.join(temporary, 'review-copy.hwpx');
    await testControl('dialog/save', {path: output});
    await shortcut('Shift+s');
    await window.waitForSelector('#reviewDialog[open]');
    assert.equal(await window.locator('#confirmSave').isDisabled(), true);
    await window.locator('#acknowledge').check();
    await window.locator('#confirmSave').click();
    await window.waitForFunction(() => !document.getElementById('reviewDialog').open);
    assert.ok((await fs.stat(output)).size > 0);
    assert.match(await window.locator('#toast').innerText(), /Saved review-copy.hwpx/);
    await window.locator('#toast button').click();
    // Wait for the backend's reveal effect after the click handler returns.
    await waitHost(state => state.revealed.includes(output));
    assert.equal((await testControl('state')).revealed.at(-1), output);
    await assert.rejects(invoke('shell_reveal_output', {path: '/tmp/other.hwpx'}), /arguments/);
    assert.deepEqual(await fs.readFile(source), before);
    const savedBytes = await fs.readFile(output);
    await testControl('dialog/save', {path: output});
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
  },
  'save-in-place': async () => {
    const {source} = await saveSource();

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
    await waitHost(state => state.revealed.some(file => file.includes('/.hwp-workbench/backups/')));
    await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
    await window.locator('#textEdit').fill('conflict');
    await window.locator('#applyText').click();
    await fs.writeFile(inPlace, baseBytes);
    const messageCount = (await testControl('state')).messages.length;
    await testControl('dialog/message', {answer: 2});
    await shortcut('s');
    await waitHost(state => state.messages.length > messageCount && state.messages.at(-1).buttons.length === 3);
    assert.deepEqual(await fs.readFile(inPlace), baseBytes);
    await window.locator('#changes .acts button').click();
  },
  'recents-language': async () => {
    const recentSource = path.join(temporary, 'recent-source.hwpx');
    await fs.copyFile(path.join(root, 'fixtures/generated/review-form.hwpx'), recentSource);
    await openFile(recentSource);
    await window.waitForFunction(async () => {
      const rows = await window.hwp.recent();
      return rows.length && (!document.querySelector('.page-image') || rows[0].thumbnail?.startsWith('data:image/png;base64,'));
    });

    const recent = await window.evaluate(() => window.hwp.recent());
    assert.ok(recent.length > 0 && recent.length <= 8);
    assert.ok(recent.every(item => !('path' in item) && !('file' in item) && typeof item.id === 'string'));
    if (await window.locator('.page-image').count()) {
      assert.match(recent[0].thumbnail, /^data:image\/png;base64,/);
    }
    const stored = JSON.parse(await fs.readFile(path.join(profile, 'recent.json'), 'utf8'));
    assert.equal(stored[0].id, recent[0].id);
    await assert.rejects(invoke('document_open_recent', {id: 0}), /Invalid recent/);
    await assert.rejects(invoke('document_open_dropped', {path: path.join(temporary, 'reject.txt')}), /Unsupported/);
    await command('Language');
    const language = await window.evaluate(() => document.documentElement.lang);
    assert.equal(language, 'ko');
    await window.reload();
    await window.waitForSelector('#recentItems .doc-card');
    assert.equal(await window.evaluate(() => document.documentElement.lang), language);
    await window.locator('#recentItems .doc-card').first().click();
    // The newest recent is whichever document the steps above opened last.
    await waitDocument(recent[0].name);
  },
  'agent-inbox': async () => {
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
  },
  'handoff': async () => {
    const inboxSource = path.join(temporary, 'handoff-start.hwpx');
    await fs.copyFile(path.join(root, 'fixtures/generated/review-form.hwpx'), inboxSource);
    await openFile(inboxSource);
    await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
    const text = await window.locator('#textEdit').inputValue();
    await window.locator('#textEdit').fill(text + ' staged');
    await window.locator('#applyText').click();
    await window.waitForFunction(() => document.querySelectorAll('#changes .change').length === 1);
    // Queue behind the renderer's draft write before delivering the handoff.
    await invoke('document_recent');

    const handoffSource = path.join(temporary, 'handoff.hwpx');
    await fs.copyFile(path.join(root, 'fixtures/generated/review-form.hwpx'), handoffSource);
    const messageCount = (await testControl('state')).messages.length;
    await testControl('dialog/message', {answer: 0});
    await testControl('handoff', {path: handoffSource});
    await waitHost(state => state.messages.length > messageCount);
    assert.equal(await window.locator('#filename').innerText(), path.basename(inboxSource));
    assert.equal(await window.locator('#changes .change').count(), 1);
    await testControl('dialog/message', {answer: 1});
    await testControl('handoff', {path: handoffSource});
    await waitDocument(path.basename(handoffSource));
    assert.equal(await window.locator('#changes .change').count(), 0);
  },
  'protocol-drop': async () => {

    for (const route of ['../package.json', '%2e%2e/package.json', 'unknown.xyz']) {
      const response = await fetch(`${hostInfo.url}${route}`);
      assert.notEqual(response.status, 200, route);
    }
    assert.equal((await fetch(new URL('/index.html', hostInfo.url))).status, 403);
    assert.notEqual(await window.evaluate(async () => (await fetch('unknown.xyz')).status), 200);
    await assert.rejects(invoke('shell_reveal_output', {path: '/tmp/other.hwpx'}), /arguments/);
    await assert.rejects(invoke('document_open_dropped', {path: '/tmp/not-granted.hwpx'}), /Unsupported/);
    const dropped = path.join(temporary, 'os-drop.hwpx');
    await fs.copyFile(path.join(root, 'fixtures/generated/review-form.hwpx'), dropped);
    await testControl('drop', {paths: [dropped]});
    await waitDocument(path.basename(dropped));
    await assert.rejects(invoke('document_open_dropped', {path: dropped}), /Unsupported/);
    assert.deepEqual(errors, []);
  },
};

const args = process.argv.slice(2);
if (args.length === 1 && args[0] === '--list') {
  console.log(Object.keys(workflows).join('\n'));
} else {
  const only = args[0] === '--only' && args.length === 2 ? args[1] : null;
  if ((args.length && !only) || (only && !workflows[only])) {
    console.error(`Usage: test-webkit.mjs [--list | --only <name>]\n${Object.keys(workflows).join(', ')}`);
    process.exitCode = 2;
  } else {
    try {
      temporary = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-webkit-')));
      profile = path.join(temporary, 'profile');
      await buildFrontend({testBuild: true});
      hostInfo = await startHost();
      browser = await webkit.launch({headless: true});
      const context = await browser.newContext({viewport: {width: 1440, height: 900}});
      context.on('page', page => {
        page.on('pageerror', error => errors.push(error.message));
        page.on('console', message => {pageConsole.push(`${message.type()}: ${message.text().replaceAll(hostInfo.token, '[token]')}`); if (pageConsole.length > 1000) pageConsole.shift();});
        page.on('dialog', dialog => dialog.accept().catch(() => {}));
      });
      window = await context.newPage();
      window.setDefaultTimeout(30000);
      await window.goto(new URL('index.html', hostInfo.url).href);
      await window.waitForSelector('#emptyOpen');
      await window.evaluate(() => {localStorage.setItem('hwp-language', 'en'); localStorage.setItem('hwp-theme', 'light');});
      await window.reload();
      await window.waitForSelector('#emptyOpen');
      for (const [name, workflow] of Object.entries(workflows)) {
        if (only && only !== name) {skipped.push(`${name}: not selected by --only`); continue;}
        currentWorkflow = name;
        console.error(`WebKit workflow: ${name}`);
        await window.evaluate(() => {localStorage.setItem('hwp-language', 'en'); localStorage.setItem('hwp-theme', 'light'); localStorage.setItem('hwp-key-scheme', 'windows');});
        await window.reload();
        await window.waitForSelector('#emptyOpen');
        await workflow();
        checks.push(name);
      }
      currentWorkflow = 'screenshots';
      // Capture the main document, review rail and Korean shell in both themes.
      await sample();
      await window.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
      const text = await window.locator('#textEdit').inputValue();
      await window.locator('#textEdit').fill(text + ' 검토');
      await window.locator('#applyText').click();
      await waitPreviewText('검토');
      const screenshots = path.join(root, 'work/qa/webkit');
      await fs.mkdir(screenshots, {recursive: true});
      for (const theme of ['Light', 'Dark']) {
        await command(`Theme ${theme}`);
        await window.screenshot({path: path.join(screenshots, `review-${theme.toLowerCase()}.png`)});
      }
      await command('Language');
      for (const theme of ['light', 'dark']) {
        await window.evaluate(theme => {localStorage.setItem('hwp-theme', theme);}, theme);
        await window.reload();
        await window.waitForSelector('#emptyOpen');
        await window.locator('#demo').click();
        await waitDocument('review-form.hwpx');
        assert.equal(await window.evaluate(() => document.documentElement.lang), 'ko');
        assert.equal(await window.evaluate(() => document.documentElement.dataset.theme), theme);
        await window.screenshot({path: path.join(screenshots, `korean-${theme}.png`)});
      }
      assert.deepEqual(errors, []);
      console.log(JSON.stringify({host: `${process.platform}/${process.arch}`, browser: 'WebKit', checks, skipped, notTested}, null, 2));
    } catch (error) {
      for (const name of Object.keys(workflows)) {
        if (!checks.includes(name) && !skipped.some(reason => reason.startsWith(`${name}:`))) {
          skipped.push(`${name}: did not complete after failure`);
        }
      }
      console.error(`Workflow failed: ${currentWorkflow ?? 'startup/screenshots'}\n${error.stack}`);
      if (window && !window.isClosed()) {
        console.error('Renderer state:', await window.evaluate(() => ({
          status: document.getElementById('status')?.textContent,
          structure: document.querySelectorAll('.structure-page').length,
          pages: document.querySelectorAll('.page-image').length,
          marks: document.querySelectorAll('.control-mark').length,
          focus: document.activeElement?.id,
          idleCallback: typeof requestIdleCallback
        })).catch(() => null));
      }
      console.error(`Page console:\n${pageConsole.join('\n')}\nPage errors:\n${errors.join('\n')}\nHost log:\n${hostLog.join('')}`);
      console.log(JSON.stringify({host: `${process.platform}/${process.arch}`, browser: 'WebKit', checks, skipped, notTested,
        failure: {workflow: currentWorkflow ?? 'startup/screenshots', message: error.message}}, null, 2));
      process.exitCode = 1;
    } finally {
      await browser?.close().catch(() => {});
      if (host && host.exitCode === null && host.signalCode === null) {
        const exited = new Promise(resolve => host.once('exit', resolve));
        host.kill('SIGTERM');
        const timer = setTimeout(() => host.kill('SIGKILL'), 3000);
        await exited;
        clearTimeout(timer);
      }
      // Restore production assets even after browser/host startup fails.
      if (temporary) await buildFrontend().catch(error => {console.error(error.message); process.exitCode = 1;});
      if (temporary) await fs.rm(temporary, {recursive: true, force: true});
    }
  }
}
