// What the floating window of the video scopes can ask of the shell, and nothing else of the system:
// what it measures, its own scopes of the engine and their pictures, and to be closed.
const { contextBridge, ipcRenderer } = require('electron');
const subscribe = (channel, callback) => {
  const listener = (_event, value) => callback(value);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
};
contextBridge.exposeInMainWorld('columbinaScopes', {
  context: () => ipcRenderer.invoke('columbina-scopes:context'),
  onContext: (callback) => subscribe('columbina-scopes:context', callback),
  call: (method, params) => ipcRenderer.invoke('columbina-scopes:call', { method, params }),
  onEvent: (callback) => subscribe('columbina-scopes:event', callback),
  close: () => ipcRenderer.invoke('columbina-scopes:close'),
});
