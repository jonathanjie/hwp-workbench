// Native macOS qualification. RUN ONLY WHILE NOBODY USES THE MAC.
// Importing this module has no effects. Tests import only its pure functions.
import {execFileSync} from 'node:child_process';
import fs from 'node:fs/promises';
import {constants} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const KOREAN = 'com.apple.inputmethod.Korean.2SetKorean';
const KEY = {a: 0, s: 1, f: 3, g: 5, q: 12, r: 15, k: 40, m: 46, enter: 36, backspace: 51, escape: 53};
const FIXTURES = ['review-form.hwpx', 'legacy-review.hwp'];
export const PREFLIGHT_COMMANDS = [['preflight']];

export function parseArguments(args, root = ROOT) {
  const options = {app: path.join(root, 'dist-mac/HWP Workbench.app'), helper: null,
    dryRun: false, help: false, rounds: 3, timeout: 30};
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--dry-run') options.dryRun = true;
    else if (arg === '--help') options.help = true;
    else if (['--app', '--helper', '--rounds', '--timeout'].includes(arg)) {
      const value = args[++index];
      if (!value || value.startsWith('--')) throw new Error(`Missing value for ${arg}`);
      const key = arg.slice(2);
      if (key === 'app' || key === 'helper') options[key] = path.resolve(value);
      else {
        const number = Number(value);
        const maximum = key === 'rounds' ? 10 : 120;
        if (!Number.isInteger(number) || number < 1 || number > maximum) {
          throw new Error(`${arg} must be an integer from 1 to ${maximum}`);
        }
        options[key] = number;
      }
    } else throw new Error(`Unknown argument: ${arg}`);
  }
  return options;
}

export function compareResult(actual, expected) {
  return Object.entries(expected).every(([key, value]) =>
    Object.hasOwn(actual, key) && JSON.stringify(actual[key]) === JSON.stringify(value));
}

export function preflightFailures(state) {
  const failures = [];
  const requirements = [
    [state.platform === 'darwin', 'macOS is required'],
    [state.screenUnlocked === true, 'Screen is locked or session state is unavailable'],
    [state.trusted === true, 'Accessibility permission is missing'],
    [state.koreanAvailable === true, 'Enabled 2-Set Korean input source is missing'],
    [typeof state.currentSource === 'string' && state.currentSource.length > 0, 'Cannot record the original input source'],
    [state.appExists === true, 'Packaged Tauri app is missing'],
    [state.cliExists === true, 'Bundled CLI is missing or is not executable'],
    [state.fixturesExist === true, 'The two generated sample fixtures are missing'],
    [Array.isArray(state.instances) && state.instances.length === 0, 'Another HWP Workbench instance is running']
  ];
  for (const [passed, reason] of requirements) if (!passed) failures.push(reason);
  return failures;
}

export function driverPlan() {
  return [
    'Read screen-lock, Accessibility, input-source, package, fixture and instance preconditions',
    'Copy the two self-authored samples to a fresh temporary workspace',
    'Launch the packaged app with the HWPX sample and measure its on-screen window and render times',
    'Measure app and attributable WebKit helper memory after render',
    'Run real 2-Set Korean composition, recomposition, Backspace and Enter rounds in the slip and bare control (if exposed)',
    'Stage 한글 with Cmd+Enter; check that the slip closes and one edit contains the exact text',
    'Save with Cmd+S and acknowledge the app review sheet; verify text, changed hash and original backup with the bundled CLI',
    'Open the HWP sample through the running instance; check its filename and process count',
    'Restore the input source and quit; confirm all owned processes exit; print JSON with failures and notTested'
  ];
}

export function nativeCheckNames(rounds) {
  const names = ['on-screen window', 'first document rendered', 'memory measured'];
  for (let round = 1; round <= rounds; round += 1) {
    for (const target of ['control', 'slip']) names.push(`${target}: 한글 round ${round}`);
  }
  for (const target of ['control', 'slip']) {
    names.push(`${target}: 하나 recomposition`, `${target}: jamo Backspace`);
  }
  return [...names, 'control: Enter commits', 'slip: Enter commits, no line break',
    'slip: Cmd+Enter during composition (WebKit ordering outcome)',
    'save: newText', 'save: changedHash', 'save: originalBackup', 'save: sameFormat',
    'second open uses running instance', 'second fixture bytes retained',
    'input source restored', 'app quit normally', 'all owned processes exited'];
}

export function unexecutedChecks(planned, checks, notTested) {
  const reported = new Set([...checks.map(item => item.name), ...notTested.map(item => item.check)]);
  return planned.filter(name => !reported.has(name)).map(check => ({check, required: true,
    reason: 'Run stopped before this check could execute'}));
}

export function parseProcesses(output) {
  return output.split('\n').flatMap(line => {
    const match = /^\s*(\d+)\s+(\d+)\s+(\d+)\s+(.+?)\s*$/.exec(line);
    return match ? [{pid: Number(match[1]), parentPid: Number(match[2]), rssBytes: Number(match[3]) * 1024, command: match[4]}] : [];
  });
}

export function ownedProcesses(rows, appPid, responsible = new Map()) {
  const owned = new Set([appPid]);
  let added;
  do {
    added = false;
    for (const row of rows) {
      if (!owned.has(row.pid) && (owned.has(row.parentPid) || responsible.get(row.pid) === appPid)) {
        owned.add(row.pid);
        added = true;
      }
    }
  } while (added);
  return rows.filter(row => owned.has(row.pid));
}

export function parseFootprint(output) {
  const match = /^\s*phys_footprint:\s*(\d+)\s+B\s*$/m.exec(output);
  return match ? Number(match[1]) : null;
}

export function saveChecks({beforeHash, afterHash, beforeText, afterText, expectedText, backupHashes, beforeFormat, afterFormat}) {
  return {
    newText: !!expectedText && !beforeText.includes(expectedText) && afterText.includes(expectedText),
    changedHash: beforeHash !== afterHash,
    originalBackup: backupHashes.includes(beforeHash),
    sameFormat: ['hwp', 'hwpx'].includes(beforeFormat) && beforeFormat === afterFormat
  };
}

class CommandError extends Error {
  constructor(command, cause) {
    const stderr = String(cause.stderr || cause.message).trim();
    super(`${command}: ${stderr}`);
    this.code = null;
    try { this.code = JSON.parse(stderr).code; } catch { /* Non-helper commands use plain stderr. */ }
  }
}

function run(command, args, timeout = 30000) {
  try {
    return execFileSync(command, args.map(String), {encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
      timeout, maxBuffer: 8 * 1024 * 1024});
  } catch (error) { throw new CommandError(command, error); }
}
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
async function exists(file, mode = constants.F_OK) {
  try { await fs.access(file, mode); return true; } catch { return false; }
}
function processes() { return parseProcesses(run('/bin/ps', ['-axo', 'pid=,ppid=,rss=,comm='])); }
function isWorkbench(row) { return ['HWP Workbench', 'hwp-workbench', 'Electron'].includes(path.basename(row.command)) &&
  (path.basename(row.command) !== 'Electron' || row.command.includes('HWP Workbench.app')); }
function isWebKit(row) { return /com\.apple\.WebKit\.(?:WebContent|GPU|Networking)(?:\b|$)/.test(row.command); }

async function until(description, seconds, read) {
  const deadline = performance.now() + seconds * 1000;
  let lastError;
  do {
    try {
      const value = await read();
      if (value) return value;
    } catch (error) {
      // A missing element is expected during load. Permission, limits and
      // unreadable trees must not turn into successful absence checks.
      if (!['AX_NOT_FOUND', 'AX_BUTTON_MATCH', 'AX_NO_FOCUS'].includes(error.code)) throw error;
      lastError = error;
    }
    await pause(150);
  } while (performance.now() < deadline);
  throw new Error(`Timed out waiting for ${description}${lastError ? `: ${lastError.message}` : ''}`);
}

async function preflight(options, helper) {
  const cli = path.join(options.app, 'Contents/Resources/bin/hwp-workbench');
  const state = {platform: process.platform, instances: [], screenUnlocked: null, trusted: false,
    koreanAvailable: false, currentSource: '',
    appExists: await exists(path.join(options.app, 'Contents/MacOS/HWP Workbench'), constants.X_OK),
    cliExists: await exists(cli, constants.X_OK),
    fixturesExist: (await Promise.all(FIXTURES.map(name => exists(path.join(ROOT, 'fixtures/generated', name))))).every(Boolean)};
  if (process.platform === 'darwin') {
    Object.assign(state, JSON.parse(run(helper, PREFLIGHT_COMMANDS[0])));
    const instancePids = new Set(state.instances.map(row => row.pid));
    for (const row of processes().filter(isWorkbench)) {
      if (!instancePids.has(row.pid)) state.instances.push(row);
    }
  }
  return {state, failures: preflightFailures(state)};
}

async function memorySample(appPid, axPids, report, remember) {
  const rows = processes();
  const responsible = new Map();
  for (const row of rows) {
    if (isWebKit(row) && axPids.includes(row.pid)) responsible.set(row.pid, appPid);
  }
  // launchd commonly owns XPC helpers. procinfo can identify responsibility,
  // but some hosts require root. Never use sudo or count all WebKit processes.
  for (const row of rows.filter(row => isWebKit(row) && !responsible.has(row.pid))) {
    try {
      const info = run('/bin/launchctl', ['procinfo', row.pid], 3000);
      const match = /responsible\s+pid\s*[:=]\s*(\d+)/i.exec(info);
      if (match) responsible.set(row.pid, Number(match[1]));
    } catch { /* Unavailable attribution is reported below, not guessed. */ }
  }
  const owned = ownedProcesses(rows, appPid, responsible);
  remember(owned);
  const unassigned = rows.filter(row => isWebKit(row) && !owned.some(item => item.pid === row.pid) && !responsible.has(row.pid));
  const measured = [];
  for (const row of owned.filter(row => row.pid === appPid || isWebKit(row))) {
    let bytes = null;
    let reason = null;
    try { bytes = parseFootprint(run('/usr/bin/footprint', ['-f', 'bytes', '-p', row.pid], 10000)); }
    catch (error) { reason = error.message; }
    measured.push({...row, physicalFootprintBytes: bytes, method: bytes === null ? 'ps RSS (not physical footprint)' : 'footprint -f bytes', reason});
  }
  const complete = unassigned.length === 0 && measured.length > 0 && measured.every(row => row.physicalFootprintBytes !== null);
  if (!complete) report.notTested.push({check: 'complete physical footprint', required: true,
    reason: 'Some footprint readings or WebKit ownership records are unavailable. RSS is a separate fallback; unrelated helpers are excluded.',
    unattributedWebKitPids: unassigned.map(row => row.pid)});
  return {method: 'footprint -f bytes per owned PID; ps RSS fallback; parent chain / app-rooted AX PID / launchctl procinfo responsibility',
    complete, processes: measured, unattributedWebKitPids: unassigned.map(row => row.pid),
    physicalFootprintBytes: complete ? measured.reduce((total, row) => total + row.physicalFootprintBytes, 0) : null,
    measuredPhysicalFootprintBytes: measured.reduce((total, row) => total + (row.physicalFootprintBytes ?? 0), 0),
    measuredRssBytes: measured.reduce((total, row) => total + row.rssBytes, 0)};
}

async function qualify(options, helper, initial, report) {
  const cli = path.join(options.app, 'Contents/Resources/bin/hwp-workbench');
  let workspace = null;
  let pid = null;
  let launched = false;
  let windowId = null;
  let sourceMayHaveChanged = false;
  let phase = 'prepare fixture copies and CLI read';
  const owned = new Map();
  const ime = (...args) => JSON.parse(run(helper, args, (options.timeout + 10) * 1000));
  const ax = (command, id) => ime(command, pid, ...(id === undefined ? [] : [id]));
  const value = id => ax('ax-value', id);
  // Plain text spans (file name, format, page counter) have no AX DOM id in WebKit.
  const texts = () => ime('ax-texts', pid);
  const PAGE_COUNTER = /^\d+\s*\/\s*[1-9]\d*$/;
  // The change count is a span inside the Changes tab; WebKit exposes it in the tab title.
  const changeCount = () => /(\d+)\s*$/.exec(String(ax('ax-find', 'changesTab').title ?? ''))?.[1] ?? '';
  const remember = rows => {
    for (const row of rows) {
      if (!owned.has(row.pid)) {
        const started = run('/bin/ps', ['-p', row.pid, '-o', 'lstart=']).trim();
        owned.set(row.pid, {...row, started});
      }
    }
  };
  const stillOwned = row => {
    try {
      return run('/bin/ps', ['-p', row.pid, '-o', 'lstart=']).trim() === row.started &&
        processes().some(current => current.pid === row.pid && current.command === row.command);
    } catch { return false; }
  };
  const appRows = () => processes().filter(row =>
    row.command === path.join(options.app, 'Contents/MacOS/HWP Workbench'));
  // Keep key encoding separate so every batch has an explicit focus guard.
  const type = (field, ...names) => ime('guarded-keys', pid, field ?? '-', ...names.map(name => {
    const [key, ...modifiers] = name.split(':');
    if (!Object.hasOwn(KEY, key)) throw new Error(`Unknown key: ${key}`);
    return [KEY[key], ...modifiers].join(':');
  }));
  const present = id => {
    try { return ax('ax-find', id); }
    catch (error) { if (error.code === 'AX_NOT_FOUND') return null; throw error; }
  };
  const record = (name, passed, details = {}) => {
    report.checks.push({name, passed, ...details});
    console.error(`${passed ? 'PASS' : 'FAIL'} ${name}`);
  };
  const closeSlip = async () => {
    if (present('textEdit')) {
      ax('ax-click', 'textEdit');
      type('textEdit', 'escape');
      await until('slip close', options.timeout, () => !present('textEdit'));
    }
  };
  const clear = async id => {
    // Blur first so old composition commits before Select All and Backspace.
    ax('ax-click', 'zoomValue');
    await pause(300);
    ax('ax-click', id);
    await until(`${id} focus`, options.timeout, () => ax('ax-focused').domId === id);
    type(id, 'a:cmd', 'backspace');
    await until(`${id} clear`, options.timeout, () => value(id) === '');
    await pause(300);
  };
  const rendered = async (filename, observe = () => {}) => {
    await until(`document ${filename}`, options.timeout, () => {
      observe();
      return texts().includes(filename);
    });
    await until('page counter', options.timeout, () => {
      observe();
      return texts().some(item => PAGE_COUNTER.test(item.trim()));
    });
  };

  try {
    // The engine compares paths with the canonical workspace root, and macOS
    // temporary folders sit behind the /var -> /private/var symlink.
    workspace = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-native-qa-')));
    report.workspace = workspace;
    const files = FIXTURES.map(name => path.join(workspace, name));
    for (let index = 0; index < FIXTURES.length; index += 1) {
      await fs.copyFile(path.join(ROOT, 'fixtures/generated', FIXTURES[index]), files[index], constants.COPYFILE_EXCL);
    }
    report.fixtures = [];
    for (const file of files) report.fixtures.push({name: path.basename(file), sha256: hash(await fs.readFile(file)), provenance: 'self-authored synthetic parser fixture'});
    const beforeHash = hash(await fs.readFile(files[0]));
    const beforeText = run(cli, ['read', files[0], '--root', workspace, '--format', 'text']);
    const document = JSON.parse(run(cli, ['inspect', files[0], '--root', workspace, '--compact']));
    const node = document.nodes?.find(item => item.editable && item.text);
    if (!node) throw new Error('The HWPX sample has no editable nonempty text node');
    const failures = (await preflight(options, helper)).failures;
    if (failures.length) throw new Error(`Preflight changed before launch: ${failures.join('; ')}`);
    phase = 'on-screen window';
    const start = performance.now();
    launched = true; // Cleanup still runs if open starts the app then returns an error.
    run('/usr/bin/open', ['-n', '-a', options.app, files[0]]);
    pid = await until('one packaged app process', options.timeout, () => {
      const rows = appRows();
      for (const row of rows) remember(ownedProcesses(processes(), row.pid));
      if (rows.length > 1) throw new Error('More than one packaged app process started');
      return rows[0]?.pid;
    });
    report.pid = pid;
    remember(ownedProcesses(processes(), pid));
    const window = ime('wait-window', pid, options.timeout);
    windowId = window.id;
    report.timings.windowSeconds = (performance.now() - start) / 1000;
    record('on-screen window', true, {window, elapsedSeconds: report.timings.windowSeconds});
    phase = 'first document rendered';
    await rendered(FIXTURES[0]);
    await until('editable text rendered', options.timeout, () => ax('ax-find-text', node.text).enabled);
    report.timings.renderSeconds = (performance.now() - start) / 1000;
    report.renderEvidence = {method: 'AX page counter and editable button', structureFallback: !!present('structureChip')};
    record('first document rendered', true, {elapsedSeconds: report.timings.renderSeconds});
    phase = 'memory measured';
    report.memory = await memorySample(pid, ime('ax-processes', pid), report, remember);
    record('memory measured', report.memory.processes.some(row => row.pid === pid), {complete: report.memory.complete});

    phase = 'select Korean input source';
    sourceMayHaveChanged = true;
    run(helper, ['select', KOREAN]);
    if (run(helper, ['current']).trim() !== KOREAN) throw new Error('Cannot select 2-Set Korean input');
    record('select Korean input source', true);
    await pause(1000);
    const controlElement = present('imeControl');
    const control = controlElement?.role === 'AXTextArea' && controlElement.enabled;
    report.control = {available: !!control, element: controlElement};
    const openSlip = async (label = node.text) => {
      if (present('textEdit')) return;
      await until('editable text button', options.timeout, () => ax('ax-find-text', label).enabled);
      ax('ax-click-text', label);
      await until('proof slip', options.timeout, () => present('textEdit'));
    };
    const check = async (name, target, sequence, expected) => {
      phase = `${target}: ${name}`;
      const id = target === 'control' ? 'imeControl' : 'textEdit';
      if (target === 'control' && !control) {
        report.notTested.push({check: `${target}: ${name}`, required: true, reason: 'Packaged app does not expose an enabled bare #imeControl textarea; AX cannot inject one'});
        return;
      }
      if (target === 'slip') await openSlip();
      await clear(id);
      type(id, ...sequence);
      await pause(600);
      const actual = {value: value(id)};
      record(`${target}: ${name}`, compareResult(actual, expected), {expected, actual});
    };
    for (let round = 1; round <= options.rounds; round += 1) {
      for (const target of ['control', 'slip']) await check(`한글 round ${round}`, target, ['g', 'k', 's', 'r', 'm', 'f'], {value: '한글'});
    }
    for (const target of ['control', 'slip']) {
      await check('하나 recomposition', target, ['g', 'k', 's', 'k'], {value: '하나'});
      await check('jamo Backspace', target, ['g', 'k', 's', 'k', 'backspace'], {value: '하ㄴ'});
    }
    await check('Enter commits', 'control', ['g', 'k', 's', 'r', 'm', 'f', 'enter'], {value: '한글\n'});
    await check('Enter commits, no line break', 'slip', ['g', 'k', 's', 'r', 'm', 'f', 'enter'], {value: '한글'});
    phase = 'slip: Cmd+Enter during composition (WebKit ordering outcome)';
    await openSlip();
    await clear('textEdit');
    type('textEdit', 'g', 'k', 's', 'r', 'm', 'f', 'enter:cmd');
    await until('composition edit staged and slip closed', options.timeout, () => !present('textEdit') && changeCount() === '1');
    // Reading the reopened slip checks the model's staged text, not an AX
    // concatenation of removed and inserted diff fragments.
    await openSlip('한글');
    const stagedText = value('textEdit');
    await closeSlip();
    const actual = {value: present('textEdit') ? value('textEdit') : null, staged: [stagedText]};
    record('slip: Cmd+Enter during composition (WebKit ordering outcome)',
      compareResult(actual, {value: null, staged: ['한글']}) && changeCount() === '1',
      {expected: {value: null, staged: ['한글']}, actual});

    phase = 'save flow';
    const saveStart = performance.now();
    type(null, 's:cmd');
    let confirmed = false;
    await until('save or review sheet', options.timeout, async () => {
      if (hash(await fs.readFile(files[0])) !== beforeHash) return true;
      const confirm = present('confirmSave');
      if (confirm?.frame?.width > 0 && confirm.frame.height > 0 && !confirmed) {
        // The sheet slides in, and a click during that animation can miss the
        // checkbox. Click, then wait for the tick, so a slow click never toggles twice.
        await pause(500);
        const ticked = () => [true, 1].includes(value('acknowledge'));
        for (let attempt = 0; attempt < 4 && !ticked(); attempt += 1) {
          ax('ax-click', 'acknowledge');
          await until('acknowledgement ticked', 1.5, ticked).catch(() => false);
        }
        await until('enabled save confirmation', options.timeout, () => present('confirmSave')?.enabled);
        ax('ax-click', 'confirmSave');
        confirmed = true;
      }
      return false;
    });
    await rendered(FIXTURES[0]);
    // With no staged edits the side panel closes and its Changes tab leaves the
    // Accessibility tree; then the toolbar must show no edit counter either.
    const noStagedEdits = () => {
      try {
        return changeCount() === '0';
      } catch (error) {
        if (error.code !== 'AX_NOT_FOUND') throw error;
        return !texts().some(item => /^\d+\s*edits?$|^수정\s*\d+$/i.test(item.trim()));
      }
    };
    await until('save reopened and staged edits cleared', options.timeout, noStagedEdits);
    report.timings.saveSeconds = (performance.now() - saveStart) / 1000;
    const afterHash = hash(await fs.readFile(files[0]));
    const afterText = run(cli, ['read', files[0], '--root', workspace, '--format', 'text']);
    const backupFolder = path.join(workspace, '.hwp-workbench/backups');
    const backups = [];
    if (await exists(backupFolder)) {
      for (const entry of await fs.readdir(backupFolder, {withFileTypes: true})) {
        if (entry.isFile()) backups.push({name: entry.name, sha256: hash(await fs.readFile(path.join(backupFolder, entry.name)))});
      }
    }
    const afterDocument = JSON.parse(run(cli, ['inspect', files[0], '--root', workspace, '--compact']));
    const checks = saveChecks({beforeHash, afterHash, beforeText, afterText, expectedText: '한글',
      backupHashes: backups.map(item => item.sha256), beforeFormat: document.format, afterFormat: afterDocument.format});
    for (const [name, passed] of Object.entries(checks)) record(`save: ${name}`, passed, {beforeHash, afterHash, backups, confirmed});

    phase = 'second open uses running instance';
    const secondStart = performance.now();
    const originalPid = pid;
    const observations = [];
    const observe = () => {
      const rows = appRows();
      observations.push(rows.map(row => row.pid));
      for (const row of rows) remember(ownedProcesses(processes(), row.pid));
    };
    observe();
    run('/usr/bin/open', ['-a', options.app, files[1]]);
    await rendered(FIXTURES[1], observe);
    const secondDocument = JSON.parse(run(cli, ['inspect', files[1], '--root', workspace, '--compact']));
    const secondNode = secondDocument.nodes?.find(item => item.editable && item.text);
    if (!secondNode) throw new Error('Second sample has no editable text to confirm render');
    await until('second document text rendered', options.timeout, () => {
      observe();
      return ax('ax-find-text', secondNode.text).enabled && texts().some(item => item.trim().toLowerCase() === secondDocument.format);
    });
    // Record repeated process observations during and after the handoff.
    for (let sample = 0; sample < 10; sample += 1) {
      observations.push(appRows().map(row => row.pid));
      await pause(100);
    }
    record('second open uses running instance', observations.every(pids => pids.length === 1 && pids[0] === originalPid),
      {filenameShown: texts().includes(FIXTURES[1]), observations});
    record('second fixture bytes retained', hash(await fs.readFile(files[1])) === report.fixtures[1].sha256);
    report.timings.secondOpenSeconds = (performance.now() - secondStart) / 1000;
    remember(ownedProcesses(processes(), pid));
  } catch (error) {
    record(phase, false, {reason: error.message});
    if (windowId !== null && workspace) {
      const screenshot = path.join(workspace, 'failure.png');
      try {
        run('/usr/sbin/screencapture', ['-o', '-x', '-l', String(windowId), screenshot]);
        report.failureScreenshot = screenshot;
      } catch { /* The window may already be gone; the recorded reason stands. */ }
    }
  } finally {
    // Each cleanup step is independent. A failed restore must not leave the app.
    if (sourceMayHaveChanged) {
      try {
        run(helper, ['select', initial.currentSource]);
        record('input source restored', run(helper, ['current']).trim() === initial.currentSource);
      } catch (error) { record('input source restored', false, {reason: error.message}); }
    }
    if (launched) {
      try {
        if (!pid) pid = appRows()[0]?.pid;
        if (pid) {
          remember(ownedProcesses(processes(), pid));
          let graceful = false;
          try {
            if (!stillOwned(owned.get(pid))) throw new Error('App PID identity changed before quit');
            type(null, 'q:cmd');
            for (let attempt = 0; attempt < 20; attempt += 1) {
              const current = processes();
              if (!current.some(row => row.pid === pid)) { graceful = true; break; }
              // This is the app's explicit close choice for driver-owned edits.
              try { ax('ax-click-text', 'Close / 닫기'); }
              catch (error) { if (!['AX_BUTTON_MATCH', 'PROCESS_MISSING', 'AX_READ_FAILED'].includes(error.code)) throw error; }
              await pause(150);
            }
          } catch (error) { report.cleanupQuitReason = error.message; }
          record('app quit normally', graceful);
          // An unsaved prompt or lost foreground must not retain our app.
          // Only PIDs with their recorded start time and executable are signalled.
          for (const signal of ['SIGTERM', 'SIGKILL']) {
            for (const row of owned.values()) {
              if (stillOwned(row)) {
                try { process.kill(row.pid, signal); }
                catch (error) { if (error.code !== 'ESRCH') throw error; }
              }
            }
            await pause(500);
          }
          const survivors = [...owned.values()].filter(stillOwned);
          const remainingApps = appRows().map(row => row.pid);
          if (report.memory && !report.memory.complete) {
            report.notTested.push({check: 'unattributed WebKit helper exit', required: true,
              reason: 'Unknown helper ownership prevents a complete helper exit check; no unrelated process was signalled'});
          }
          record('all owned processes exited', survivors.length === 0 && remainingApps.length === 0, {survivors: survivors.map(row => row.pid), remainingApps, trackedPids: [...owned.keys()]});
        } else record('all owned processes exited', appRows().length === 0);
      } catch (error) { record('app cleanup', false, {reason: error.message}); }
    }
    // Retain copied fixtures and backups as evidence. Never delete them while
    // an app might still have the files open; the summary prints their path.
    report.elapsedSeconds = (performance.now() - report.startedAt) / 1000;
    delete report.startedAt;
  }
}

async function main() {
  const report = {host: {platform: process.platform, arch: process.arch, release: os.release(), node: process.version},
    app: null, dryRun: false, checks: [], timings: {}, memory: null, notTested: [], startedAt: performance.now()};
  let helperFolder = null;
  try {
    const options = parseArguments(process.argv.slice(2));
    if (options.help) {
      console.log('Usage: npm run qa:native -- [--app PATH] [--helper PATH] [--dry-run] [--rounds 1..10] [--timeout 1..120]\nRun only while nobody uses this Mac. --dry-run reads state and posts no events.');
      return;
    }
    if (await exists(options.app)) options.app = await fs.realpath(options.app);
    report.app = options.app;
    report.dryRun = options.dryRun;
    report.plannedChecks = nativeCheckNames(options.rounds);
    report.notTested.push({check: 'public-file / Hancom layout and reopen', reason: 'This driver uses only the two self-authored synthetic samples'});
    report.notTested.push({check: 'composition event trace', reason: 'AX reads values and staging outcomes; it cannot observe compositionend / keydown event order'});
    let helper = options.helper;
    if (!helper && process.platform === 'darwin') {
      helperFolder = await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-native-helper-'));
      helper = path.join(helperFolder, 'ime-keys');
      run('/usr/bin/swiftc', ['-O', '-warnings-as-errors', path.join(ROOT, 'scripts/qa/ime-keys.swift'), '-o', helper], 60000);
    }
    const {state, failures} = await preflight(options, helper);
    report.preflight = state;
    report.checks.push({name: 'preflight', passed: failures.length === 0, failures});
    if (options.dryRun) {
      report.plan = driverPlan();
      report.notTested.push({check: 'all native checks', reason: 'Dry run: no app launch, clicks, keys, source selection or quit'});
    } else if (!failures.length) await qualify(options, helper, state, report);
    else report.notTested.push({check: 'all native checks', required: true, reason: 'Preflight refused to start'});
  } catch (error) { report.checks.push({name: 'driver setup', passed: false, reason: error.message}); }
  finally {
    if (helperFolder) {
      try { await fs.rm(helperFolder, {recursive: true, force: true}); }
      catch (error) { report.checks.push({name: 'helper cleanup', passed: false, reason: error.message}); }
    }
  }
  if (report.plannedChecks && !report.dryRun) {
    report.notTested.push(...unexecutedChecks(report.plannedChecks, report.checks, report.notTested));
  }
  report.passed = report.checks.filter(check => check.passed).length;
  report.failed = report.checks.filter(check => !check.passed).length;
  report.complete = !report.dryRun && report.failed === 0 && !report.notTested.some(item => item.required);
  if (report.startedAt !== undefined) {
    report.elapsedSeconds = (performance.now() - report.startedAt) / 1000;
    delete report.startedAt;
  }
  console.log(JSON.stringify(report, null, 2));
  process.exitCode = report.failed || report.notTested.some(item => item.required) ? 1 : 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
