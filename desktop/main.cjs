const {app, BrowserWindow, dialog, ipcMain, session, clipboard, Menu, protocol, nativeTheme, shell} = require('electron');
const {spawn} = require('node:child_process');
const fs = require('node:fs/promises');
const fsSync = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const {serveAppRequest} = require('./protocol.cjs');
const {RecentStore, validateDroppedFile} = require('./recent-store.cjs');
const {OutputReveal} = require('./output-reveal.cjs');
const {OpenHandoff, openArgument, requestedOpen} = require('./open-handoff.cjs');
const {AgentTools} = require('./agent-tools.cjs');
const pythonProcess = require('./python-process.cjs');
const {pythonExecutable, engineSpawnOptions} = pythonProcess;
const {AgentInbox} = require('./agent-inbox.cjs');
const {readDocumentBytes} = require('./document-bytes.cjs');

const {installMenu} = require('./menu.cjs');
const {LatestPreview} = require('./preview-lane.cjs');
const previews = new LatestPreview();

const ROOT = path.resolve(__dirname, '..');
pythonProcess.useElectron(require('electron'));
const INDEX = 'app://workbench/index.html';
// Honor Chromium's explicit profile-directory option for isolated app runs.
const profileDirectory = app.commandLine.getSwitchValue('user-data-dir');
if (profileDirectory) {
  app.setPath('userData', path.resolve(profileDirectory));
}
const initialOpen = openArgument(process.argv);
const primaryInstance = app.requestSingleInstanceLock({open: initialOpen});
if (!primaryInstance) {
  app.quit();
}
let rendererReady = false;
let handoffActive = false;
const pendingOpen = initialOpen ? [initialOpen] : [];
let handoff;
let editing = false;
let win;
let current = null;
const agentTools = new AgentTools({
  documentRoot: () => current?.root,
  copy: text => clipboard.writeText(text),
  reveal: file => shell.showItemInFolder(file)
});
const previousReveal = new OutputReveal(file => shell.showItemInFolder(file));
const reviewedFiles = new Set();
const outputReveal = new OutputReveal(file => shell.showItemInFolder(file));
let queue = Promise.resolve();
let recentStore;
let agentInbox;
let stagedPatch = null;
let selection = null;
function recents() {
  const directory = app.getPath('userData');
  if (!recentStore || recentStore.directory !== directory) {
    recentStore = new RecentStore(directory);
  }
  return recentStore;
}

protocol.registerSchemesAsPrivileged([{scheme: 'app', privileges: {
  standard: true,
  secure: true,
  supportFetchAPI: true,
  corsEnabled: true
}}]);

function callEngine(root, method, args, signal) {
  const payload = JSON.stringify({method, args});
  if (Buffer.byteLength(payload) > 4 * 1024 * 1024) {
    return Promise.reject(Error('Request too large'));
  }
  return new Promise((resolve, reject) => {
    const child = spawn(pythonExecutable(), ['-m', 'hwp_workbench', 'rpc', '--root', root], engineSpawnOptions());
    let output = '';
    let errorOutput = '';
    let size = 0;
    let settled = false;
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      finish(Error('Processing timed out'));
    }, 30000);
    function finish(error, value) {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      if (error) {
        reject(error);
      } else {
        resolve(value);
      }
    }
    signal?.addEventListener('abort', () => {
      child.kill('SIGKILL');
      finish(Error('Preview dropped'));
    }, {once: true});
    child.on('error', () => finish(Error('Python unavailable')));
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', data => {
      size += Buffer.byteLength(data);
      if (size > (method === 'preview' ? 90 : 32) * 1024 * 1024) {
        child.kill('SIGKILL');
        finish(Error('Response too large'));
      } else {
        output += data;
      }
    });
    child.stderr.on('data', data => {
      if (errorOutput.length < 65536) {
        errorOutput += data;
      }
    });
    child.stdin.on('error', () => {});
    child.on('close', code => {
      if (code !== 0) {
        let message = 'Engine failed';
        try {
          const engineError = JSON.parse(errorOutput).error;
          const error = Error(engineError.message);
          error.code = engineError.code;
          finish(error);
          return;
        } catch {}
        finish(Error(message));
        return;
      }
      try {
        finish(null, JSON.parse(output));
      } catch {
        finish(Error('Invalid engine response'));
      }
    });
    child.stdin.end(payload);
  });
}

function guard(event) {
  if (!win || event.sender !== win.webContents || event.senderFrame !== win.webContents.mainFrame || event.senderFrame.url !== INDEX) {
    throw Error('Unauthorized IPC sender');
  }
}

function validatePatch(patch) {
  if (!patch || patch.schemaVersion !== 1 || !current || patch.baseSha256 !== current.sha256 ||
      !Array.isArray(patch.operations) || patch.operations.length > 1000) {
    throw Error('Invalid or stale patch');
  }
  if (Buffer.byteLength(JSON.stringify(patch)) > 2 * 1024 * 1024) {
    throw Error('Patch too large');
  }
  return patch;
}

// Key drafts by file and content: a copied file with the same bytes must not
// offer another file's staged edits.
function draftPath() {
  const file = crypto.createHash('sha256').update(current.file).digest('hex').slice(0, 32);
  return path.join(app.getPath('userData'), 'drafts', `${file}-${current.sha256}.json`);
}

// Recent files are a convenience. A failure to record one must not block
// opening the document; the welcome screen then shows its own short status.
async function recordRecent(file, format) {
  try {
    return await recents().record(file, format);
  } catch (error) {
    console.error('Recent files unavailable:', error.message);
    return null;
  }
}

async function load(file) {
  previews.cancel();
  file = await fs.realpath(file);
  const root = path.dirname(file);
  const document = await callEngine(root, 'inspect', {path: path.basename(file)});
  const recentId = await recordRecent(file, document.format);
  const baseBytes = await readDocumentBytes(file, document.sha256);
  current = {file, root, baseBytes, name: document.name, format: document.format, sha256: document.sha256, recentId};
  stagedPatch = null;
  editing = false;
  selection = null;
  if (agentInbox) {
    await agentInbox.setDocument(current);
  }
  outputReveal.clear();
  previousReveal.clear();
  let draft = null;
  try {
    draft = JSON.parse(await fs.readFile(draftPath(), 'utf8'));
  } catch {}
  return {document, draft, recentId, saveReviewed: reviewedFiles.has(file), agentLink: agentInbox?.state()};
}

function register(channel, fn) {
  ipcMain.handle(channel, (event, ...args) => {
    guard(event);
    const task = queue.then(() => fn(...args));
    queue = task.catch(() => {});
    return task;
  });
}

register('document:open', async () => {
  const result = await dialog.showOpenDialog(win, {
    properties: ['openFile'],
    filters: [{name: 'HWP / HWPX', extensions: ['hwp', 'hwpx']}]
  });
  return result.canceled ? null : load(result.filePaths[0]);
});
// The preload grants paths only from trusted OS file drops. Recheck the grant
// before passing it to the engine; directories and symlinks are not documents.
register('document:open-dropped', async file => load(await validateDroppedFile(file)));
register('document:demo', () => load(path.join(ROOT, 'fixtures', 'generated', 'review-form.hwpx')));
register('document:bytes', (...args) => {
  if (args.length || !current) {
    throw Error('No document');
  }
  return readDocumentBytes(current.file, current.sha256);
});
register('document:recent', () => recents().list());
register('document:open-recent', async id => load(await recents().fileFor(id)));
register('document:cache-thumbnail', (id, png) => {
  if (!current || id !== current.recentId) {
    throw Error('Invalid thumbnail document');
  }
  return recents().cacheThumbnail(id, png);
});
register('document:format', nodeId => {
  if (!current || (nodeId !== null && typeof nodeId !== 'string')) {
    throw Error('Invalid format selection');
  }
  return callEngine(current.root, 'format', {path: path.basename(current.file), nodeId});
});
register('document:plan', patch => {
  validatePatch(patch);
  return callEngine(current.root, 'plan', {path: path.basename(current.file), patch});
});
ipcMain.handle('document:preview', (event, patch) => {
  guard(event);
  validatePatch(patch);
  const document = current;
  return previews.run(signal => callEngine(document.root, 'preview', {
    path: path.basename(document.file), patch
  }, signal));
});
register('document:save', async (patch, acknowledgement) => {
  validatePatch(patch);
  if (acknowledgement !== true) {
    throw Error('Compatibility acknowledgement required');
  }
  const extension = path.extname(current.file);
  const defaultPath = current.file.slice(0, -extension.length) + '-edited' + extension;
  const result = await dialog.showSaveDialog(win, {
    defaultPath,
    filters: [{name: 'HWP / HWPX', extensions: [extension.slice(1)]}]
  });
  if (result.canceled) {
    return null;
  }
  // Only the native dialog grants an output path. The engine refuses overwrites.
  const output = path.resolve(result.filePath);
  // Save As uses the opened revision, even if another app changed disk bytes.
  // The native dialog grants this destination folder. The private snapshot is
  // removed after the engine verifies and publishes a new, non-overwriting file.
  const root = await fs.realpath(path.dirname(output));
  const temporary = await fs.mkdtemp(path.join(root, '.hwp-workbench-copy-'));
  let report;
  try {
    const snapshot = path.join(temporary, path.basename(current.file));
    await fs.writeFile(snapshot, current.baseBytes, {flag: 'wx', mode: 0o600});
    report = await callEngine(root, 'apply', {
      path: path.relative(root, snapshot), output: path.basename(output),
      patch, acknowledgeRisks: true
    });
  } finally {
    await fs.rm(temporary, {recursive: true, force: true});
  }
  outputReveal.saved(output);
  return {...report, name: path.basename(output)};
});
register('document:save-in-place', async (patch, acknowledgement) => {
  validatePatch(patch);
  const file = current.file;
  if (!reviewedFiles.has(file) && acknowledgement !== true) {
    throw Error('Compatibility acknowledgement required');
  }
  // The acknowledgement belongs to this document session, including a
  // cancelled conflict dialog. It does not need another review sheet.
  reviewedFiles.add(file);
  previews.cancel();
  const oldDraft = draftPath();
  let report;
  try {
    report = await callEngine(current.root, 'save', {
      path: path.basename(file), patch, acknowledgeRisks: true
    });
  } catch (error) {
    if (error.code === 'SOURCE_CHANGED') {
      const result = await dialog.showMessageBox(win, {
        type: 'warning', message: 'Source changed / 원본 변경됨',
        buttons: ['Reload / 불러오기 다시', 'Save As / 다른 이름으로 저장', 'Cancel / 취소'],
        defaultId: 2, cancelId: 2
      });
      if (result.response === 0) {
        return {action: 'reload', opened: await load(file)};
      }
      return {action: result.response === 1 ? 'saveAs' : 'cancel'};
    }
    if (error.code === 'READ_ONLY_DOCUMENT') {
      const result = await dialog.showMessageBox(win, {
        type: 'warning', message: 'Read-only document / 읽기 전용 문서',
        buttons: ['Save As / 다른 이름으로 저장', 'Cancel / 취소'],
        defaultId: 0, cancelId: 1
      });
      return {action: result.response === 0 ? 'saveAs' : 'cancel'};
    }
    throw error;
  }
  await fs.rm(oldDraft, {force: true});
  const opened = await load(file);
  // Only an engine-issued workspace-relative backup grants Reveal access.
  previousReveal.saved(path.resolve(current.root, report.backup));
  return {action: 'saved', opened, warnings: report.warnings};
});
register('shell:reveal-previous', (...args) => previousReveal.show(...args));
register('shell:reveal-output', (...args) => outputReveal.show(...args));
register('document:export-patch', async patch => {
  validatePatch(patch);
  const result = await dialog.showSaveDialog(win, {
    defaultPath: current.file + '.patch.json',
    filters: [{name: 'JSON patch', extensions: ['json']}]
  });
  if (result.canceled) {
    return null;
  }
  await fs.writeFile(result.filePath, JSON.stringify(patch, null, 2), {flag: 'wx', mode: 0o600});
  return true;
});
register('document:draft', async patch => {
  validatePatch(patch);
  stagedPatch = patch;
  agentInbox?.update(selection, patch);
  const destination = draftPath();
  await fs.mkdir(path.dirname(destination), {recursive: true, mode: 0o700});
  const temporary = destination + '.' + crypto.randomBytes(8).toString('hex') + '.tmp';
  try {
    await fs.writeFile(temporary, JSON.stringify(patch), {flag: 'wx', mode: 0o600});
    await fs.rename(temporary, destination);
  } finally {
    await fs.rm(temporary, {force: true});
  }
  return true;
});
register('agents:status', (...args) => agentTools.status(...args));
register('agents:install-skill', target => agentTools.install(target));
register('agents:copy-mcp', client => agentTools.copyMcp(client));
register('agents:reveal-bundle', (...args) => agentTools.bundle(...args));
register('document:handoff-ready', (...args) => {
  if (args.length) {
    throw Error('Handoff readiness accepts no arguments');
  }
  handoffActive = false;
  flushOpenRequests();
  return true;
});
register('document:editing', value => {
  if (typeof value !== 'boolean') {
    throw Error('Invalid edit state');
  }
  editing = value;
  return true;
});
register('agent:link', enabled => agentInbox.toggle(enabled));
register('agent:selection', value => {
  if (!current || (value !== null && (!value || typeof value.nodeId !== 'string' || typeof value.text !== 'string' || value.text.length > 200000))) {
    throw Error('Invalid selection');
  }
  selection = value;
  agentInbox.update(selection, stagedPatch);
  return true;
});
register('agent:decide', (file, ids, accepted) => agentInbox.decide(file, ids, accepted));
register('keyboard:scheme', scheme => {
  if (!['windows', 'mac'].includes(scheme)) {
    throw Error('Invalid keyboard scheme');
  }
  return installMenu({Menu, app, win}, scheme).then(() => true);
});
register('clipboard:copy', text => {
  if (typeof text !== 'string' || text.length > 4096) {
    throw Error('Invalid clipboard text');
  }
  clipboard.writeText(text);
  return true;
});

function focusWindow() {
  if (win?.isMinimized()) {
    win.restore();
  }
  win?.show();
  win?.focus();
}

function flushOpenRequests() {
  if (!rendererReady || !handoff || handoffActive || !pendingOpen.length) {
    return;
  }
  handoffActive = true;
  const file = pendingOpen.shift();
  const task = queue.then(async () => {
    if (!(await handoff.open(file))) {
      handoffActive = false;
      // Schedule after this queue entry settles; a cancelled open needs no
      // renderer acknowledgement because the current view did not change.
      setTimeout(flushOpenRequests, 0);
    }
  });
  queue = task.catch(error => {
    focusWindow();
    win.webContents.send('document:handoff', null, error.message);
  });
}

app.on('second-instance', (_event, argv, _directory, additionalData) => {
  const file = requestedOpen(argv, additionalData);
  if (file) {
    pendingOpen.push(file);
    flushOpenRequests();
  }
  focusWindow();
});
app.on('open-file', (event, file) => {
  event.preventDefault();
  pendingOpen.push(file);
  flushOpenRequests();
});

if (primaryInstance) {
  app.whenReady().then(() => {
    protocol.handle('app', request => serveAppRequest(request, {
      desktop: __dirname,
      vendor: path.join(ROOT, 'node_modules', '@rhwp', 'core')
    }));
    session.defaultSession.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
    session.defaultSession.setPermissionCheckHandler(() => false);
    session.defaultSession.webRequest.onBeforeRequest({urls: ['http://*/*', 'https://*/*', 'ws://*/*', 'wss://*/*']}, (_details, callback) => callback({cancel: true}));
    win = new BrowserWindow({
      width: 1400,
      height: 930,
      minWidth: 1100,
      minHeight: 720,
      title: 'HWP Workbench',
      titleBarStyle: 'hiddenInset',
      trafficLightPosition: {x: 18, y: 18},
      backgroundColor: nativeTheme.shouldUseDarkColors ? '#0F0F0E' : '#E8E7E3',
      icon: fsSync.existsSync(path.join(__dirname, 'assets', 'icon.png')) ? path.join(__dirname, 'assets', 'icon.png') : undefined,
      webPreferences: {
        preload: path.join(__dirname, 'preload.cjs'),
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
        webSecurity: true,
        allowRunningInsecureContent: false,
        spellcheck: false
      }
    });
    handoff = new OpenHandoff({
      hasStaged: () => !!stagedPatch?.operations.length || editing,
      confirm: async () => (await dialog.showMessageBox(win, {
        type: 'question', buttons: ['Keep editing / 계속 편집', 'Open document / 문서 불러오기'],
        defaultId: 0, cancelId: 0,
        message: 'Unsaved text / 저장하지 않은 텍스트'
      })).response === 1,
      load,
      complete: result => win.webContents.send('document:handoff', result),
      focus: focusWindow
    });
    win.webContents.on('did-finish-load', () => {
      rendererReady = true;
      flushOpenRequests();
    });
    win.webContents.on('did-start-loading', () => { rendererReady = false; });
    agentInbox = new AgentInbox({
      preferences: path.join(app.getPath('userData'), 'agent-links.json'),
      plan: (document, patch) => callEngine(document.root, 'plan', {path: document.name, patch}),
      send: state => win.webContents.send('agent:inbox', state)
    });
    win.webContents.setWindowOpenHandler(() => ({action: 'deny'}));
    win.webContents.on('will-navigate', event => event.preventDefault());
    win.webContents.on('will-prevent-unload', async () => {
      const result = await dialog.showMessageBox(win, {
        type: 'question',
        buttons: ['Keep editing / 계속 편집', 'Close / 닫기'],
        defaultId: 0,
        cancelId: 0,
        message: 'Unsaved text / 저장하지 않은 텍스트',
        detail: 'Staged draft retained · unstaged text unsaved / 적용된 초안 보관 · 미적용 텍스트 미저장'
      });
      if (result.response === 1) {
        win.destroy();
      }
    });
    installMenu({Menu, app, win}, 'windows');
    win.loadURL(INDEX);
  });
}
app.on('window-all-closed', () => {
  agentInbox?.stop();
  app.quit();
});
