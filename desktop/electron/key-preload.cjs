// What the window for the key can ask of the shell, and nothing else of the system.
const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('columbinaKey', {
  context: () => ipcRenderer.invoke('columbina-key:context'),
  store: (document) => ipcRenderer.invoke('columbina-key:store', document),
  check: (document) => ipcRenderer.invoke('columbina-key:check', document),
  forget: (name) => ipcRenderer.invoke('columbina-key:forget', name),
  openPage: (address) => ipcRenderer.invoke('columbina-key:open-page', address),
  close: () => ipcRenderer.invoke('columbina-key:close'),
});
