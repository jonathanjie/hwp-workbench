const {contextBridge,ipcRenderer}=require('electron');
// Narrow capabilities only; never expose ipcRenderer, fs, shell or exec.
contextBridge.exposeInMainWorld('hwp',Object.freeze({
 open:()=>ipcRenderer.invoke('document:open'),
 openDemo:()=>ipcRenderer.invoke('document:demo'),
 plan:(patch)=>ipcRenderer.invoke('document:plan',patch),
 saveCopy:(patch,ack)=>ipcRenderer.invoke('document:save',patch,ack),
 exportPatch:(patch)=>ipcRenderer.invoke('document:export-patch',patch),
 saveDraft:(patch)=>ipcRenderer.invoke('document:draft',patch),
 copyText:(text)=>ipcRenderer.invoke('clipboard:copy',text)
}));
