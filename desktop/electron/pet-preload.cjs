// What the window of the pet can ask of the shell, and nothing else of the system.
const { contextBridge, ipcRenderer } = require('electron');
const subscribe = (channel, callback) => {
  const listener = (_event, value) => callback(value);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
};
contextBridge.exposeInMainWorld('columbinaPet', {
  ready: () => ipcRenderer.invoke('columbina-pet:ready'),
  onShown: (callback) => subscribe('columbina-pet:shown', callback),
  onSheet: (callback) => subscribe('columbina-pet:sheet', callback),
  click: () => ipcRenderer.invoke('columbina-pet:click'),
  drag: (phase) => ipcRenderer.invoke('columbina-pet:drag', phase),
  menu: () => ipcRenderer.invoke('columbina-pet:menu'),
});
