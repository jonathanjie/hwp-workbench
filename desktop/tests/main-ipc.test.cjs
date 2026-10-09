const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const vm = require('node:vm');
const {createRequire} = require('node:module');

const desktop = path.resolve(__dirname, '..');
const root = path.dirname(desktop);

// Test the production handlers with a small Electron host double and the real
// Python engine. This is IPC wiring evidence, not native Electron qualification.
async function host(directory, source) {
  const handlers = new Map();
  const revealed = [];
  let window;
  const dialog = {
    showOpenDialog: async () => ({canceled: false, filePaths: [source]}),
    showSaveDialog: async () => ({canceled: false, filePath: path.join(directory, 'copy.hwpx')})
  };
  class BrowserWindow {
    constructor() {
      this.webContents = {
        mainFrame: {url: 'app://workbench/index.html'},
        setWindowOpenHandler() {},
        on() {},
        send() {}
      };
      window = this;
    }
    loadURL() {}
  }
  const electron = {
    app: {name: 'Test', commandLine: {getSwitchValue: () => ''}, getPath: () => directory, whenReady: () => Promise.resolve(), on() {}},
    BrowserWindow, dialog,
    ipcMain: {handle: (channel, handler) => handlers.set(channel, handler)},
    session: {defaultSession: {setPermissionRequestHandler() {}, setPermissionCheckHandler() {}, webRequest: {onBeforeRequest() {}}}},
    protocol: {registerSchemesAsPrivileged() {}, handle() {}},
    nativeTheme: {shouldUseDarkColors: false},
    Menu: {buildFromTemplate: value => value, setApplicationMenu() {}},
    shell: {showItemInFolder: file => revealed.push(file)},
    clipboard: {writeText() {}}
  };
  const localRequire = createRequire(path.join(desktop, 'main.cjs'));
  vm.runInNewContext(await fs.readFile(path.join(desktop, 'main.cjs'), 'utf8'), {
    require: module => module === 'electron' ? electron : localRequire(module),
    __dirname: desktop, process, Buffer, console, setTimeout, clearTimeout
  });
  await new Promise(resolve => setImmediate(resolve));
  const event = {sender: window.webContents, senderFrame: window.webContents.mainFrame};
  return {handlers, event, dialog, revealed, invoke: (channel, ...args) => handlers.get(channel)(event, ...args)};
}

test('main IPC enforces sender, id, drop, acknowledgement, reveal and non-overwrite boundaries', async () => {
  const directory = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-ipc-')));
  try {
    const source = path.join(directory, 'source.hwpx');
    const original = await fs.readFile(path.join(root, 'fixtures/generated/review-form.hwpx'));
    await fs.writeFile(source, original);
    const app = await host(directory, source);
    assert.throws(() => app.handlers.get('document:recent')({sender: {}, senderFrame: {url: 'app://workbench/index.html'}}), /Unauthorized/);
    await assert.rejects(app.invoke('shell:reveal-output'), /No saved copy/);
    await assert.rejects(app.invoke('document:open-dropped', path.join(directory, 'bad.txt')), /Unsupported/);
    const opened = await app.invoke('document:open');
    const recent = await app.invoke('document:recent');
    assert.equal(recent[0].id, opened.recentId);
    assert.equal(Object.hasOwn(recent[0], 'file'), false);
    await assert.rejects(app.invoke('document:open-recent', 0), /Invalid recent/);
    assert.equal((await app.invoke('document:open-recent', opened.recentId)).document.sha256, opened.document.sha256);
    await assert.rejects(app.invoke('document:cache-thumbnail', 'wrong', 'data:image/png;base64,'), /Invalid thumbnail document/);
    await assert.rejects(app.invoke('document:bytes', source), /No document/);
    const node = opened.document.nodes.find(node => node.editable);
    const patch = {schemaVersion: 1, baseSha256: opened.document.sha256, operations: [
      {op: 'set_text', nodeId: node.id, expectedText: node.text, text: node.text + ' 검토'}
    ]};
    await assert.rejects(app.invoke('document:save', patch, false), /acknowledgement/);
    await app.invoke('document:plan', patch);
    const saved = await app.invoke('document:save', patch, true);
    assert.equal(saved.name, 'copy.hwpx');
    await assert.rejects(app.invoke('shell:reveal-output', '/tmp/other'), /arguments/);
    await app.invoke('shell:reveal-output');
    assert.deepEqual(app.revealed, [path.join(directory, 'copy.hwpx')]);
    const output = await fs.readFile(path.join(directory, 'copy.hwpx'));
    await assert.rejects(app.invoke('document:save', patch, true), /exist/i);
    assert.deepEqual(await fs.readFile(path.join(directory, 'copy.hwpx')), output);
    assert.deepEqual(await fs.readFile(source), original);
    await app.invoke('document:open-recent', opened.recentId);
    await assert.rejects(app.invoke('shell:reveal-output'), /No saved copy/);
  } finally {
    await fs.rm(directory, {recursive: true, force: true});
  }
});
