// What the window for the key can ask of the shell, and nothing else of the system.
const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('columbinaKey', {
  context: () => ipcRenderer.invoke('columbina-key:context'),
  store: (document) => ipcRenderer.invoke('columbina-key:store', document),
  close: () => ipcRenderer.invoke('columbina-key:close'),
});
