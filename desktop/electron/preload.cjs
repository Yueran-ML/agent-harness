const { contextBridge, ipcRenderer } = require('electron');
const subscribe = (channel, callback) => {
  const listener = (_event, value) => callback(value);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
};
contextBridge.exposeInMainWorld('columbina', {
  selectFolder: () => ipcRenderer.invoke('columbina:select-folder'),
  getLocalMusic: () => ipcRenderer.invoke('columbina:local-music'),
  start: (options) => ipcRenderer.invoke('columbina:start', { cwd: options?.cwd }),
  send: (line) => ipcRenderer.send('columbina:send', line),
  onLine: (callback) => subscribe('columbina:line', callback),
  onClose: (callback) => subscribe('columbina:closed', callback),
  onShutdown: (callback) => subscribe('columbina:shutdown', callback),
  stop: () => ipcRenderer.invoke('columbina:stop'),
  setTheme: (theme) => ipcRenderer.invoke('columbina:set-theme', theme),
  openExternal: (url) => ipcRenderer.invoke('columbina:open-external', url),
});
