import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import vm from 'node:vm';

const source = await fs.readFile(new URL('../tauri-bridge.js', import.meta.url), 'utf8').catch(() => '');
const preloadSource = await fs.readFile(new URL('../preload.cjs', import.meta.url), 'utf8');

function preload() {
  const calls = [];
  let api;
  const electron = {
    contextBridge: {exposeInMainWorld: (name, value) => {
      assert.equal(name, 'hwp');
      api = value;
    }},
    ipcRenderer: {
      invoke: async (channel, ...args) => {
        calls.push({channel, args});
        return 'result';
      },
      on: () => {},
      removeListener: () => {}
    },
    webUtils: {getPathForFile: file => file.path}
  };
  vm.runInNewContext(preloadSource, {require: name => {
    assert.equal(name, 'electron');
    return electron;
  }});
  return {api, calls};
}

function bridge(options = {}) {
  const listeners = new Map();
  const calls = [];
  const removed = [];
  const window = {__TAURI__: {
    core: {invoke: async (command, args) => {
      calls.push({command, args});
      if (options.failure) throw options.failure;
      return options.results?.[command] ?? 'result';
    }},
    event: {listen: (event, listener) => {
      listeners.set(event, listener);
      return options.registration ?? Promise.resolve(() => removed.push(event));
    }}
  }};
  vm.runInNewContext(source, {window, console});
  assert.ok(window.hwp, 'bridge must install window.hwp');
  return {api: window.hwp, calls, removed, listeners};
}

test('Tauri bridge preserves the exact preload API and command argument names', async () => {
  const {api, calls} = bridge({results: {document_bytes: [0, 127, 255]}});
  const reference = preload();
  const methods = {
    keyScheme: ['keyboard_scheme', {scheme: 'windows'}, ['windows']],
    agentsStatus: ['agents_status', undefined, []],
    installSkill: ['agents_install_skill', {target: 'claude'}, ['claude']],
    copyMcp: ['agents_copy_mcp', {client: 'codex'}, ['codex']],
    revealBundle: ['agents_reveal_bundle', undefined, []],
    handoffReady: ['document_handoff_ready', undefined, []],
    editing: ['document_editing', {value: true}, [true]],
    agentLink: ['agent_link', {enabled: true}, [true]],
    agentSelection: ['agent_selection', {selection: {nodeId: 'n1', text: '한글'}}, [{nodeId: 'n1', text: '한글'}]],
    agentDecide: ['agent_decide', {file: 'proposal', ids: ['n1'], accepted: true}, ['proposal', ['n1'], true]],
    open: ['document_open', undefined, []],
    openDemo: ['document_demo', undefined, []],
    documentBytes: ['document_bytes', undefined, []],
    recent: ['document_recent', undefined, []],
    openRecent: ['document_open_recent', {id: 'one'}, ['one']],
    cacheThumbnail: ['document_cache_thumbnail', {id: 'one', png: 'png'}, ['one', 'png']],
    preview: ['document_preview', {patch: {}}, [{}]],
    format: ['document_format', {nodeId: 'n1'}, ['n1']],
    plan: ['document_plan', {patch: {}}, [{}]],
    save: ['document_save_in_place', {patch: {}, acknowledgement: true}, [{}, true]],
    showPrevious: ['shell_reveal_previous', undefined, []],
    saveCopy: ['document_save', {patch: {}, acknowledgement: true}, [{}, true]],
    showSaved: ['shell_reveal_output', undefined, []],
    exportPatch: ['document_export_patch', {patch: {}}, [{}]],
    saveDraft: ['document_draft', {patch: {}}, [{}]],
    copyText: ['clipboard_copy', {text: '한글'}, ['한글']]
  };
  assert.deepEqual(Object.keys(api).sort(), Object.keys(reference.api).sort());
  assert.deepEqual(Object.keys(api).sort(), [...Object.keys(methods), 'onCommand', 'onHandoff', 'onAgentInbox', 'onDrop'].sort());
  assert.ok(Object.isFrozen(api));
  const registeredSource = await fs.readFile(new URL('../../app/src-tauri/src/command_names.rs', import.meta.url), 'utf8');
  const registered = [...registeredSource.matchAll(/"([a-z_]+)"/g)].map(match => match[1]);
  assert.deepEqual(registered.sort(), [...Object.values(methods).map(([command]) => command), 'document_open_dropped'].sort());
  for (const [method, [command, args, values]] of Object.entries(methods)) {
    const result = await api[method](...values);
    if (method === 'documentBytes') {
      assert.deepEqual(Array.from(result), [0, 127, 255]);
    } else {
      assert.equal(result, 'result');
    }
    assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), JSON.parse(JSON.stringify({command, args})));
    assert.equal(await reference.api[method](...values), 'result');
    const [group, ...action] = command.split('_');
    assert.deepEqual(JSON.parse(JSON.stringify(reference.calls.at(-1))), {
      channel: `${group}:${action.join('-')}`,
      args: values
    });
  }
});

test('event subscriptions validate commands and unpack handoff and inbox payloads', async () => {
  const {api, listeners} = bridge();
  const commands = [];
  api.onCommand(value => commands.push(value));
  listeners.get('app:command')({payload: 'open'});
  listeners.get('app:command')({payload: 'shell'});
  assert.deepEqual(commands, ['open']);
  const handoffs = [];
  api.onHandoff((result, error) => handoffs.push([result, error]));
  listeners.get('document:handoff')({payload: {result: null, error: 'failure'}});
  assert.deepEqual(handoffs, [[null, 'failure']]);
  const inboxes = [];
  api.onAgentInbox(state => inboxes.push(state));
  listeners.get('agent:inbox')({payload: {rows: []}});
  assert.deepEqual(inboxes, [{rows: []}]);
});

test('documentBytes returns the Uint8Array exposed by Electron', async () => {
  const {api} = bridge({results: {document_bytes: [0, 127, 255]}});
  const bytes = await api.documentBytes();
  assert.equal(Object.prototype.toString.call(bytes), '[object Uint8Array]');
  assert.deepEqual(Array.from(bytes), [0, 127, 255]);
});

test('Tauri rejection messages reach the renderer as Error messages', async () => {
  for (const failure of ['Source changed', {message: 'Source changed'}]) {
    const {api} = bridge({failure});
    await assert.rejects(api.open(), error => error.message === 'Source changed');
  }
});

test('unsubscribe before async registration removes the listener and ignores callbacks', async () => {
  let resolve;
  const registration = new Promise(done => { resolve = done; });
  const {api, listeners} = bridge({registration});
  let calls = 0;
  const unsubscribe = api.onCommand(() => calls++);
  unsubscribe();
  unsubscribe();
  listeners.get('app:command')({payload: 'open'});
  let removed = 0;
  resolve(() => removed++);
  await registration;
  await Promise.resolve();
  assert.equal(removed, 1);
  assert.equal(calls, 0);
});

test('handoff readiness waits until the handoff event listener is installed', async () => {
  let resolve;
  const registration = new Promise(done => { resolve = done; });
  const {api, calls} = bridge({registration});
  api.onHandoff(() => {});
  const ready = api.handoffReady();
  await Promise.resolve();
  assert.deepEqual(calls, []);
  resolve(() => {});
  await ready;
  assert.equal(calls[0].command, 'document_handoff_ready');
});

test('trusted backend drop event opens only one consented path and reports failures', async () => {
  const {api, listeners, calls} = bridge({failure: 'Cannot open'});
  const completed = [];
  let consent = false;
  api.onDrop(async () => consent, (result, error) => completed.push([result, error]));
  const drop = listeners.get('tauri://drag-drop');
  await drop({payload: {paths: ['/one', '/two']}});
  await drop({payload: {paths: ['/one']}});
  assert.deepEqual(calls, []);
  consent = true;
  await drop({payload: {paths: ['/one']}});
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{command: 'document_open_dropped', args: {path: '/one'}}]);
  assert.deepEqual(completed, [[null, 'Cannot open']]);
});

test('unsubscribe during drop consent does not open a document', async () => {
  const {api, listeners, calls} = bridge();
  let resolve;
  const consent = new Promise(done => { resolve = done; });
  const unsubscribe = api.onDrop(() => consent, () => assert.fail('drop cancelled'));
  const pending = listeners.get('tauri://drag-drop')({payload: {paths: ['/one']}});
  unsubscribe();
  resolve(true);
  await pending;
  assert.deepEqual(calls, []);
});
