// Development-only transport. The frontend builder excludes this from shipped assets.
(() => {
  const base = new URL('.', document.baseURI);
  let stream = null;
  let streamReady = null;
  const tauri = {
    core: {invoke: async (command, args = {}) => {
      const response = await fetch(new URL(`invoke/${command}`, base).href, {
        method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(args)
      });
      const value = await response.json();
      if (!response.ok) throw value;
      return value;
    }},
    event: {listen: (event, callback) => {
      if (!stream) {
        stream = new EventSource(new URL('events', base).href);
        streamReady = new Promise((resolve, reject) => {
          stream.onopen = resolve;
          stream.onerror = () => reject(new Error('Test host event stream unavailable'));
        });
      }
      const listener = message => callback({payload: JSON.parse(message.data)});
      stream.addEventListener(event, listener);
      return streamReady.then(() => () => stream.removeEventListener(event, listener));
    }}
  };
  const invoke = async (command, args) => {
    try {
      return await tauri.core.invoke(command, args);
    } catch (failure) {
      // Tauri rejects with JSON values; Electron exposes an Error to callers.
      throw new Error(failure?.message ?? String(failure));
    }
  };
  const commands = new Set(['open', 'save', 'saveCopy', 'language', 'palette', 'find', 'replace', 'shortcuts', 'undo', 'redo']);
  let handoffRegistration = Promise.resolve();

  function subscribe(event, callback) {
    let active = true;
    let removeListener = null;
    const ready = tauri.event.listen(event, message => {
      if (active) {
        return callback(message.payload, () => active);
      }
    }).then(remove => {
      if (active) {
        removeListener = remove;
      } else {
        remove();
      }
    });
    // Registration errors remain available to handoffReady without creating
    // an unhandled rejection for APIs that return a synchronous unsubscribe.
    ready.catch(() => {});
    return {ready, unsubscribe: () => {
      active = false;
      removeListener?.();
      removeListener = null;
    }};
  }

  window.hwp = Object.freeze({
    onCommand: callback => subscribe('app:command', command => {
      if (commands.has(command)) {
        callback(command);
      }
    }).unsubscribe,
    keyScheme: scheme => invoke('keyboard_scheme', {scheme}),
    agentsStatus: () => invoke('agents_status'),
    installSkill: target => invoke('agents_install_skill', {target}),
    copyMcp: client => invoke('agents_copy_mcp', {client}),
    revealBundle: () => invoke('agents_reveal_bundle'),
    handoffReady: async () => {
      await handoffRegistration;
      return invoke('document_handoff_ready');
    },
    editing: value => invoke('document_editing', {value}),
    onHandoff: callback => {
      const subscription = subscribe('document:handoff', payload => callback(payload.result, payload.error));
      handoffRegistration = subscription.ready;
      return subscription.unsubscribe;
    },
    agentLink: enabled => invoke('agent_link', {enabled}),
    agentSelection: selection => invoke('agent_selection', {selection}),
    agentDecide: (file, ids, accepted) => invoke('agent_decide', {file, ids, accepted}),
    onAgentInbox: callback => subscribe('agent:inbox', callback).unsubscribe,
    open: () => invoke('document_open'),
    onDrop: (acceptDrop, completeDrop) => subscribe('tauri://drag-drop', async (payload, active) => {
      // Only backend OS drop events carry a path grant. DOM drops cannot grant
      // paths, and document_open_dropped consumes the backend grant once.
      if (!Array.isArray(payload?.paths) || payload.paths.length !== 1) {
        return;
      }
      const path = payload.paths[0];
      if (typeof path !== 'string' || !path || !(await acceptDrop()) || !active()) {
        return;
      }
      let result = null;
      let error = null;
      try {
        result = await invoke('document_open_dropped', {path});
      } catch (failure) {
        error = failure?.message ?? String(failure);
      }
      if (active()) {
        await completeDrop(result, error);
      }
    }).unsubscribe,
    openDemo: () => invoke('document_demo'),
    documentBytes: async () => new Uint8Array(await invoke('document_bytes')),
    recent: () => invoke('document_recent'),
    openRecent: id => invoke('document_open_recent', {id}),
    cacheThumbnail: (id, png) => invoke('document_cache_thumbnail', {id, png}),
    preview: patch => invoke('document_preview', {patch}),
    format: nodeId => invoke('document_format', {nodeId}),
    plan: patch => invoke('document_plan', {patch}),
    save: (patch, acknowledgement) => invoke('document_save_in_place', {patch, acknowledgement}),
    showPrevious: () => invoke('shell_reveal_previous'),
    saveCopy: (patch, acknowledgement) => invoke('document_save', {patch, acknowledgement}),
    showSaved: () => invoke('shell_reveal_output'),
    exportPatch: patch => invoke('document_export_patch', {patch}),
    saveDraft: patch => invoke('document_draft', {patch}),
    copyText: text => invoke('clipboard_copy', {text})
  });
})();
