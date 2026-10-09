const {app, BrowserWindow, dialog, ipcMain, session, clipboard, Menu, protocol, nativeTheme, shell} = require('electron');
const {spawn} = require('node:child_process');
const fs = require('node:fs/promises');
const fsSync = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const {serveAppRequest} = require('./protocol.cjs');
const {readDocumentBytes} = require('./document-bytes.cjs');

const ROOT = path.resolve(__dirname, '..');
const INDEX = 'app://workbench/index.html';
let win;
let current = null;
let savedOutput = null;
let queue = Promise.resolve();
const recentFiles = [];

protocol.registerSchemesAsPrivileged([{scheme: 'app', privileges: {
  standard: true,
  secure: true,
  supportFetchAPI: true,
  corsEnabled: true
}}]);

function callEngine(root, method, args) {
  const payload = JSON.stringify({method, args});
  if (Buffer.byteLength(payload) > 4 * 1024 * 1024) {
    return Promise.reject(Error('Request too large'));
  }
  return new Promise((resolve, reject) => {
    const python = process.env.HWP_WORKBENCH_PYTHON ||
      ['/opt/homebrew/bin/python3', '/usr/local/bin/python3', '/usr/bin/python3'].find(file => fsSync.existsSync(file)) || 'python3';
    const child = spawn(python, ['-m', 'hwp_workbench', 'rpc', '--root', root], {
      cwd: ROOT,
      shell: false,
      env: {...process.env, PYTHONPATH: path.join(ROOT, 'engine'), PYTHONUTF8: '1'},
      stdio: ['pipe', 'pipe', 'pipe']
    });
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
    child.on('error', () => finish(Error('Python unavailable')));
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', data => {
      size += Buffer.byteLength(data);
      if (size > 32 * 1024 * 1024) {
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
          message = JSON.parse(errorOutput).error.message;
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

function draftPath() {
  return path.join(app.getPath('userData'), 'drafts', current.sha256 + '.json');
}

async function load(file) {
  const root = path.dirname(file);
  const document = await callEngine(root, 'inspect', {path: path.basename(file)});
  current = {file, root, sha256: document.sha256};
  savedOutput = null;
  const index = recentFiles.indexOf(file);
  if (index !== -1) {
    recentFiles.splice(index, 1);
  }
  recentFiles.unshift(file);
  recentFiles.length = Math.min(recentFiles.length, 3);
  let draft = null;
  try {
    draft = JSON.parse(await fs.readFile(draftPath(), 'utf8'));
  } catch {}
  return {document, draft};
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
// Only preload's trusted OS drop listener sends this grant; no path API is exposed.
register('document:drop', file => {
  if (typeof file !== 'string' || !path.isAbsolute(file) || !/\.hwpx?$/i.test(file)) {
    throw Error('Unsupported document');
  }
  return load(file);
});
register('document:demo', () => load(path.join(ROOT, 'fixtures', 'generated', 'review-form.hwpx')));
register('document:bytes', (...args) => {
  if (args.length || !current) {
    throw Error('No document');
  }
  return readDocumentBytes(current.file, current.sha256);
});
register('document:recent', () => recentFiles.map(file => ({name: path.basename(file)})));
register('document:open-recent', index => {
  if (!Number.isInteger(index) || index < 0 || index >= recentFiles.length) {
    throw Error('Invalid recent document');
  }
  return load(recentFiles[index]);
});
register('document:plan', patch => {
  validatePatch(patch);
  return callEngine(current.root, 'plan', {path: path.basename(current.file), patch});
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
  const from = path.resolve(current.file).split(path.sep);
  const to = output.split(path.sep);
  let index = 0;
  while (index < from.length - 1 && index < to.length - 1 && from[index] === to[index]) {
    index += 1;
  }
  const root = from.slice(0, index).join(path.sep) || path.parse(current.file).root;
  const report = await callEngine(root, 'apply', {
    path: path.relative(root, current.file),
    output: path.relative(root, output),
    patch,
    acknowledgeRisks: true
  });
  savedOutput = output;
  return {...report, output, name: path.basename(output)};
});
register('document:show-saved', () => {
  if (!savedOutput) {
    throw Error('No saved copy');
  }
  shell.showItemInFolder(savedOutput);
  return true;
});
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
register('clipboard:copy', text => {
  if (typeof text !== 'string' || text.length > 4096) {
    throw Error('Invalid clipboard text');
  }
  clipboard.writeText(text);
  return true;
});

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
    icon: path.join(__dirname, 'assets', 'icon.svg'),
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
  Menu.setApplicationMenu(Menu.buildFromTemplate([
    {label: app.name, submenu: [{role: 'about'}, {type: 'separator'}, {role: 'quit'}]},
    {label: 'Edit / 편집', submenu: [{role: 'undo'}, {role: 'redo'}, {type: 'separator'}, {role: 'cut'}, {role: 'copy'}, {role: 'paste'}, {role: 'selectAll'}]},
    {label: 'View / 보기', submenu: [{role: 'togglefullscreen'}]}
  ]));
  win.loadURL(INDEX);
});
app.on('window-all-closed', () => app.quit());
