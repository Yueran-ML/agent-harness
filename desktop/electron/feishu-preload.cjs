// What the window for the app of Feishu can ask of the shell, and nothing else of the system.
const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('columbinaFeishu', {
  context: () => ipcRenderer.invoke('columbina-feishu:context'),
  store: (document) => ipcRenderer.invoke('columbina-feishu:store', document),
  openPage: (address) => ipcRenderer.invoke('columbina-feishu:open-page', address),
  close: () => ipcRenderer.invoke('columbina-feishu:close'),
});
