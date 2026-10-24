// What the window for the bot of QQ can ask of the shell, and nothing else of the system.
const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('columbinaQq', {
  context: () => ipcRenderer.invoke('columbina-qq:context'),
  store: (document) => ipcRenderer.invoke('columbina-qq:store', document),
  openPage: (address) => ipcRenderer.invoke('columbina-qq:open-page', address),
  close: () => ipcRenderer.invoke('columbina-qq:close'),
});
