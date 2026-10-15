const { contextBridge, ipcRenderer } = require('electron');
const subscribe = (channel, callback) => {
  const listener = (_event, value) => callback(value);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
};
contextBridge.exposeInMainWorld('columbina', {
  // Where the page draws the bar of the window, and which keys it names, go by the system.
  platform: process.platform,
  selectFolder: (asked) => ipcRenderer.invoke('columbina:select-folder', { language: asked?.language }),
  getLocalMusic: () => ipcRenderer.invoke('columbina:local-music'),
  start: (options) => ipcRenderer.invoke('columbina:start', { cwd: options?.cwd }),
  send: (line) => ipcRenderer.send('columbina:send', line),
  onLine: (callback) => subscribe('columbina:line', callback),
  onClose: (callback) => subscribe('columbina:closed', callback),
  onShutdown: (callback) => subscribe('columbina:shutdown', callback),
  onFullScreen: (callback) => subscribe('columbina:full-screen', callback),
  stop: () => ipcRenderer.invoke('columbina:stop'),
  setTheme: (theme) => ipcRenderer.invoke('columbina:set-theme', theme),
  serviceTail: () => ipcRenderer.invoke('columbina:service-tail'),
  showLog: () => ipcRenderer.invoke('columbina:show-log'),
  notify: (message) => ipcRenderer.invoke('columbina:notify', { title: message?.title, body: message?.body }),
  recentProjects: () => ipcRenderer.invoke('columbina:recent-projects'),
  openRecent: (folder) => ipcRenderer.invoke('columbina:open-recent', folder),
  services: () => ipcRenderer.invoke('columbina:services'),
  setUpService: (asked) => ipcRenderer.invoke('columbina:set-up-service', { service: asked?.service, language: asked?.language }),
  openExternal: (url) => ipcRenderer.invoke('columbina:open-external', url),
});
