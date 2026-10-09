// Manual macOS qualification: type through the real 2-Set Korean input
// source into the proof slip and a bare control textarea.
//
// It posts real key events to the frontmost app, so nobody may type or click
// during the run (about one minute). It needs the 2-Set Korean input source
// and Accessibility permission for the terminal that runs it.
//
//   swiftc -O scripts/qa/ime-keys.swift -o "$TMPDIR/ime-keys"
//   node scripts/qa/real-ime.mjs "$TMPDIR/ime-keys"
import {_electron as electron} from 'playwright';
import {execFileSync} from 'node:child_process';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

const helper = process.argv[2];
const ROUNDS = 3;
const KOREAN = 'com.apple.inputmethod.Korean.2SetKorean';
const KEY = {g: 5, k: 40, s: 1, r: 15, m: 46, f: 3, enter: 36, backspace: 51};
const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), '../..');
const results = [];

function ime(...args) {
  return execFileSync(helper, args, {encoding: 'utf8'}).trim();
}

function keys(...names) {
  ime('keys', ...names.map(name => {
    const [key, ...modifiers] = name.split(':');
    return [KEY[key], ...modifiers].join(':');
  }));
}

const original = ime('current');
const profile = await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-ime-'));
const app = await electron.launch({args: [root, `--user-data-dir=${profile}`], cwd: root, timeout: 30000});
const win = await app.firstWindow();
const pause = ms => win.waitForTimeout(ms);

async function field(target) {
  if (target === 'control') {
    return '#imeControl';
  }
  if (!(await win.locator('.slip').count())) {
    await win.locator('.text-hit[data-node-id]:not([data-node-id=""]), .text-node').first().click();
    await win.waitForSelector('.slip');
  }
  return '#textEdit';
}

// A blur commits any composition, so clearing afterwards cannot race the IME.
async function clear(selector) {
  await win.evaluate(() => document.activeElement?.blur());
  await pause(300);
  await win.locator(selector).fill('');
  await win.locator(selector).focus();
  await pause(500);
}

async function check(name, target, sequence, expected) {
  const selector = await field(target);
  await clear(selector);
  keys(...sequence);
  await pause(600);
  const actual = await win.evaluate(selector => ({
    value: document.querySelector(selector)?.value ?? null,
    staged: [...document.querySelectorAll('#changes ins')].map(node => node.textContent)
  }), selector);
  const passed = Object.entries(expected).every(([key, value]) => JSON.stringify(actual[key]) === JSON.stringify(value));
  results.push({name, target, passed, expected, actual});
  console.log(`${passed ? 'PASS' : 'FAIL'} ${target} ${name} ${JSON.stringify(actual)}`);
}

try {
  await win.waitForSelector('#emptyOpen');
  await win.locator('#demo').click();
  await win.waitForSelector('.page-image, .structure-page', {timeout: 20000});
  await win.evaluate(() => {
    const control = document.createElement('textarea');
    control.id = 'imeControl';
    control.style.cssText = 'position:fixed;left:180px;bottom:90px;width:240px;height:60px;z-index:99';
    document.body.append(control);
  });
  await app.evaluate(({app, BrowserWindow}) => {
    app.focus({steal: true});
    BrowserWindow.getAllWindows()[0].focus();
  });
  await pause(500);
  await win.locator('#imeControl').focus();
  ime('select', KOREAN);
  await pause(1500);

  for (let round = 1; round <= ROUNDS; round += 1) {
    await check(`한글 round ${round}`, 'control', ['g', 'k', 's', 'r', 'm', 'f'], {value: '한글'});
    await check(`한글 round ${round}`, 'slip', ['g', 'k', 's', 'r', 'm', 'f'], {value: '한글'});
  }
  for (const target of ['control', 'slip']) {
    await check('하나 recomposition', target, ['g', 'k', 's', 'k'], {value: '하나'});
    await check('jamo Backspace', target, ['g', 'k', 's', 'k', 'backspace'], {value: '하ㄴ'});
  }
  await check('Enter commits', 'control', ['g', 'k', 's', 'r', 'm', 'f', 'enter'], {value: '한글\n'});
  await check('Enter commits, no line break', 'slip', ['g', 'k', 's', 'r', 'm', 'f', 'enter'], {value: '한글'});
  await check('⌘↵ during composition stages once', 'slip', ['g', 'k', 's', 'r', 'm', 'f', 'enter:cmd'], {value: null, staged: ['한글']});
} finally {
  ime('select', original);
  console.log(`input source restored: ${ime('current')}`);
  console.log(JSON.stringify({host: `${process.platform}/${process.arch}`, passed: results.filter(item => item.passed).length, total: results.length}));
  app.process().kill('SIGKILL');
  await fs.rm(profile, {recursive: true, force: true});
}
