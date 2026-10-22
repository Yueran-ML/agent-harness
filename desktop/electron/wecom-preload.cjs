// What the window for the bot of WeCom can ask of the shell, and nothing else of the system.
const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('columbinaWecom', {
  context: () => ipcRenderer.invoke('columbina-wecom:context'),
  store: (document) => ipcRenderer.invoke('columbina-wecom:store', document),
  openPage: (address) => ipcRenderer.invoke('columbina-wecom:open-page', address),
  close: () => ipcRenderer.invoke('columbina-wecom:close'),
});
