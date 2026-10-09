const {contextBridge, ipcRenderer, webUtils} = require('electron');

// Named document capabilities only: no renderer-supplied filesystem or shell paths.
contextBridge.exposeInMainWorld('hwp', Object.freeze({
  open: () => ipcRenderer.invoke('document:open'),
  onDrop: (acceptDrop, completeDrop) => {
    const listener = async event => {
      if (!event.isTrusted || event.dataTransfer?.files.length !== 1) {
        return;
      }
      event.preventDefault();
      const file = event.dataTransfer.files[0];
      const grantedPath = webUtils.getPathForFile(file);
      if (!grantedPath || !(await acceptDrop())) {
        return;
      }
      let result = null;
      let error = null;
      try {
        result = await ipcRenderer.invoke('document:drop', grantedPath);
      } catch (failure) {
        error = failure.message;
      }
      await completeDrop(result, error);
    };
    document.addEventListener('drop', listener);
    return () => document.removeEventListener('drop', listener);
  },
  openDemo: () => ipcRenderer.invoke('document:demo'),
  documentBytes: () => ipcRenderer.invoke('document:bytes'),
  recent: () => ipcRenderer.invoke('document:recent'),
  openRecent: index => ipcRenderer.invoke('document:open-recent', index),
  plan: patch => ipcRenderer.invoke('document:plan', patch),
  saveCopy: (patch, acknowledgement) => ipcRenderer.invoke('document:save', patch, acknowledgement),
  showSaved: () => ipcRenderer.invoke('document:show-saved'),
  exportPatch: patch => ipcRenderer.invoke('document:export-patch', patch),
  saveDraft: patch => ipcRenderer.invoke('document:draft', patch),
  copyText: text => ipcRenderer.invoke('clipboard:copy', text)
}));
