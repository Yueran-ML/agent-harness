const { contextBridge, ipcRenderer, webUtils } = require('electron');
/** The ids of files that go with a message, and nothing else that came with them. */
const idsOf = (ids) => (Array.isArray(ids) ? ids.filter((id) => typeof id === 'string') : []);
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
  listModels: (service) => ipcRenderer.invoke('columbina:models', service),
  chooseModel: (asked) => ipcRenderer.invoke('columbina:choose-model', { service: asked?.service, model: asked?.model }),
  chooseEffort: (asked) => ipcRenderer.invoke('columbina:choose-effort', { service: asked?.service, effort: asked?.effort }),
  chooseAttachments: (asked) => ipcRenderer.invoke('columbina:choose-attachments', { language: asked?.language }),
  // Where a file that was dropped lies is found here, from the file itself. The page hands over
  // the files and never names a path: a file that a page makes up has none, and is left out.
  attachDropped: (files) => ipcRenderer.invoke('columbina:attach-paths', Array.from(files ?? [], (file) => {
    try {
      return webUtils.getPathForFile(file);
    } catch {
      return '';
    }
  }).filter((found) => typeof found === 'string' && found !== '')),
  forgetAttachments: (ids) => ipcRenderer.invoke('columbina:forget-attachments', idsOf(ids)),
  keepAttachments: (ids) => ipcRenderer.invoke('columbina:keep-attachments', idsOf(ids)),
  setUpService: (asked) => ipcRenderer.invoke('columbina:set-up-service', { service: asked?.service, language: asked?.language }),
  openExternal: (url) => ipcRenderer.invoke('columbina:open-external', url),
  about: () => ipcRenderer.invoke('columbina:about'),
  updates: (asked) => ipcRenderer.invoke('columbina:updates', { now: asked?.now === true }),
  githubStatus: () => ipcRenderer.invoke('columbina:github-status'),
  githubSignIn: () => ipcRenderer.invoke('columbina:github-sign-in'),
  githubCancel: () => ipcRenderer.invoke('columbina:github-cancel'),
  githubSignOut: () => ipcRenderer.invoke('columbina:github-sign-out'),
  skipGitHub: (skipped) => ipcRenderer.invoke('columbina:skip-github', skipped === true),
  onGitHub: (callback) => subscribe('columbina:github', callback),
  feishuStatus: () => ipcRenderer.invoke('columbina:feishu-status'),
  feishuStart: () => ipcRenderer.invoke('columbina:feishu-start'),
  feishuStop: () => ipcRenderer.invoke('columbina:feishu-stop'),
  feishuForget: () => ipcRenderer.invoke('columbina:feishu-forget'),
  feishuLog: () => ipcRenderer.invoke('columbina:feishu-log'),
  setUpFeishu: (asked) => ipcRenderer.invoke('columbina:set-up-feishu', { language: asked?.language }),
  onFeishu: (callback) => subscribe('columbina:feishu', callback),
});
