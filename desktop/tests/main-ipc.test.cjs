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
  const appEvents = new Map();
  const windowEvents = new Map();
  const sent = [];
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
        on: (name, callback) => windowEvents.set(name, callback),
        send: (channel, ...args) => sent.push({channel, args})
      };
      window = this;
    }
    loadURL() {}
    isMinimized() { return false; }
    show() {}
    focus() {}
  }
  const electron = {
    app: {name: 'Test', commandLine: {getSwitchValue: () => ''}, getPath: () => directory, requestSingleInstanceLock: () => true, whenReady: () => Promise.resolve(), on: (name, callback) => appEvents.set(name, callback)},
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
  return {handlers, event, dialog, revealed, appEvents, windowEvents, sent, invoke: (channel, ...args) => handlers.get(channel)(event, ...args)};
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
    assert.equal((await app.invoke('document:format', node.id)).character.size, 22);
    const patch = {schemaVersion: 1, baseSha256: opened.document.sha256, operations: [
      {op: 'set_text', nodeId: node.id, expectedText: node.text, text: node.text + ' 검토'}
    ]};
    await assert.rejects(app.invoke('document:save', patch, false), /acknowledgement/);
    await app.invoke('document:plan', patch);
    const preview = await app.invoke('document:preview', patch);
    assert.ok(Buffer.from(preview.bytesBase64, 'base64').length > 0);
    await app.invoke('agent:link', true);
    await app.invoke('agent:selection', {nodeId: node.id, text: node.text});
    await app.invoke('document:draft', patch);
    const other = opened.document.nodes.find(item => item.editable && item.id !== node.id);
    const proposalPatch = {...patch, operations: [...patch.operations, {op: 'set_text', nodeId: other.id, expectedText: other.text, text: other.text + ' 제안'}]};
    const filename = 'source.hwpx.20261009T120000Z-abcdef.json';
    const folder = path.join(directory, '.hwp-workbench/proposals');
    await fs.writeFile(path.join(folder, filename), JSON.stringify({schemaVersion: 1, kind: 'hwp-workbench-proposal',
      document: {name: 'source.hwpx', sha256: opened.document.sha256}, title: 'Review', author: 'Claude',
      createdAt: '2026-10-09T12:00:00Z', patch: proposalPatch}));
    // Turning the switch on again performs a complete engine-validated scan.
    const staleFile = filename.replace('abcdef', '000001');
    const invalidFile = filename.replace('abcdef', '000002');
    const proposalRecord = JSON.parse(await fs.readFile(path.join(folder, filename), 'utf8'));
    await fs.writeFile(path.join(folder, staleFile), JSON.stringify({...proposalRecord,
      document: {...proposalRecord.document, sha256: '0'.repeat(64)}, patch: {...proposalPatch, baseSha256: '0'.repeat(64)}}));
    await fs.writeFile(path.join(folder, invalidFile), JSON.stringify({...proposalRecord,
      patch: {...proposalPatch, operations: [{...proposalPatch.operations[0], expectedText: 'stale text'}]}}));
    const inbox = await app.invoke('agent:link', true);
    assert.equal(inbox.proposals.find(row => row.file === staleFile).status, 'stale');
    assert.equal(inbox.proposals.find(row => row.file === invalidFile).status, 'unreadable');
    await assert.rejects(app.invoke('agent:decide', staleFile, [node.id], true), /Stale/);
    const applied = await app.invoke('agent:decide', filename, [node.id], true);
    assert.equal(applied.author, 'Claude');
    assert.deepEqual(applied.operations, patch.operations);
    await app.invoke('agent:decide', filename, [other.id], false);
    const outcome = JSON.parse(await fs.readFile(path.join(folder, filename.replace(/\.json$/, '.outcome.json')), 'utf8'));
    assert.equal(outcome.status, 'partial');
    assert.deepEqual(outcome.acceptedNodeIds, [node.id]);
    assert.deepEqual(outcome.declinedNodeIds, [other.id]);
    await assert.rejects(app.invoke('agent:decide', filename, [other.id], true), /Stale/);
    await app.invoke('agent:link', false);

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


test('queued handoffs wait for the renderer before switching to the next document', async () => {
  const directory = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-handoff-')));
  try {
    const first = path.join(directory, 'first.hwpx');
    const second = path.join(directory, 'second.hwpx');
    const original = await fs.readFile(path.join(root, 'fixtures/generated/review-form.hwpx'));
    await fs.writeFile(first, original);
    await fs.writeFile(second, original);
    const app = await host(directory, first);
    app.appEvents.get('second-instance')({}, ['electron', '.', '--open', first]);
    let prevented = false;
    app.appEvents.get('open-file')({preventDefault: () => { prevented = true; }}, second);
    assert.equal(prevented, true);
    assert.equal(app.sent.length, 0);
    app.windowEvents.get('did-finish-load')();
    await app.invoke('document:recent');
    assert.equal(app.sent.filter(item => item.channel === 'document:handoff').length, 1);
    await app.invoke('document:handoff-ready');
    await app.invoke('document:recent');
    assert.equal(app.sent.filter(item => item.channel === 'document:handoff').length, 2);
    await app.invoke('document:handoff-ready');
  } finally {
    await fs.rm(directory, {recursive: true, force: true});
  }
});

test('in-place save reloads its revision, retains two versions and refuses disk conflicts', async () => {
  const directory = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-save-ipc-')));
  try {
    const source = path.join(directory, 'source.hwpx');
    const original = await fs.readFile(path.join(root, 'fixtures/generated/review-form.hwpx'));
    await fs.writeFile(source, original);
    const app = await host(directory, source);
    const dialogs = [];
    app.dialog.showMessageBox = async (_window, options) => {
      dialogs.push(options);
      return {response: 2};
    };
    let opened = await app.invoke('document:open');
    const patchFor = () => {
      const node = opened.document.nodes.find(item => item.editable);
      return {schemaVersion: 1, baseSha256: opened.document.sha256, operations: [
        {op: 'set_text', nodeId: node.id, expectedText: node.text, text: node.text + ' 검토'}
      ]};
    };
    await assert.rejects(app.invoke('document:save-in-place', patchFor(), false), /acknowledgement/);
    await app.invoke('document:draft', patchFor());
    opened = (await app.invoke('document:save-in-place', patchFor(), true)).opened;
    assert.equal(opened.draft, null);
    assert.notDeepEqual(await fs.readFile(source), original);
    opened = (await app.invoke('document:save-in-place', patchFor(), false)).opened;
    assert.equal((await fs.readdir(path.join(directory, '.hwp-workbench/backups'))).length, 2);
    await assert.rejects(app.invoke('shell:reveal-previous', source), /arguments/);
    await app.invoke('shell:reveal-previous');
    assert.ok(app.revealed[0].startsWith(path.join(directory, '.hwp-workbench/backups/')));
    await fs.writeFile(source, original);
    assert.equal((await app.invoke('document:save-in-place', patchFor(), false)).action, 'cancel');
    assert.deepEqual(await fs.readFile(source), original);
    assert.equal(dialogs.length, 1);
    assert.equal(dialogs[0].buttons.length, 3);
  } finally {
    await fs.rm(directory, {recursive: true, force: true});
  }
});

test('Save As keeps opened bytes after an external change and read-only save offers a copy', async () => {
  const directory = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-copy-ipc-')));
  try {
    const source = path.join(directory, 'source.hwpx');
    const original = await fs.readFile(path.join(root, 'fixtures/generated/review-form.hwpx'));
    await fs.writeFile(source, original);
    const app = await host(directory, source);
    const opened = await app.invoke('document:open');
    const node = opened.document.nodes.find(item => item.editable);
    const patch = {schemaVersion: 1, baseSha256: opened.document.sha256, operations: [
      {op: 'set_text', nodeId: node.id, expectedText: node.text, text: node.text + ' 사본'}
    ]};
    const changed = Buffer.from('external bytes');
    await fs.writeFile(source, changed);
    await app.invoke('document:save', patch, true);
    assert.deepEqual(await fs.readFile(source), changed);
    assert.ok((await fs.stat(path.join(directory, 'copy.hwpx'))).size > 0);
    await fs.writeFile(source, original);
    await fs.chmod(source, 0o444);
    app.dialog.showMessageBox = async (_win, options) => {
      assert.equal(options.buttons.length, 2);
      return {response: 0};
    };
    assert.equal((await app.invoke('document:save-in-place', patch, true)).action, 'saveAs');
  } finally {
    await fs.chmod(path.join(directory, 'source.hwpx'), 0o600).catch(() => {});
    await fs.rm(directory, {recursive: true, force: true});
  }
});
