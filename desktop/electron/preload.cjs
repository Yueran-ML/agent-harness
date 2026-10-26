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
  startChat: () => ipcRenderer.invoke('columbina:start-chat'),
  send: (line) => ipcRenderer.send('columbina:send', line),
  onLine: (callback) => subscribe('columbina:line', callback),
  onClose: (callback) => subscribe('columbina:closed', callback),
  onShutdown: (callback) => subscribe('columbina:shutdown', callback),
  onFullScreen: (callback) => subscribe('columbina:full-screen', callback),
  stop: () => ipcRenderer.invoke('columbina:stop'),
  setTheme: (theme) => ipcRenderer.invoke('columbina:set-theme', theme),
  setLanguage: (language) => ipcRenderer.invoke('columbina:set-language', language),
  loginItem: () => ipcRenderer.invoke('columbina:login-item'),
  setLoginItem: (on) => ipcRenderer.invoke('columbina:set-login-item', on === true),
  serviceTail: () => ipcRenderer.invoke('columbina:service-tail'),
  showLog: () => ipcRenderer.invoke('columbina:show-log'),
  notify: (message) => ipcRenderer.invoke('columbina:notify', { title: message?.title, body: message?.body }),
  recentProjects: () => ipcRenderer.invoke('columbina:recent-projects'),
  openRecent: (folder) => ipcRenderer.invoke('columbina:open-recent', folder),
  services: () => ipcRenderer.invoke('columbina:services'),
  listModels: (service) => ipcRenderer.invoke('columbina:models', service),
  balance: (service) => ipcRenderer.invoke('columbina:balance', service),
  setReasoningLanguage: (language) => ipcRenderer.invoke('columbina:reasoning-language', language),
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
  // A picture that was pasted has no file: its bytes go over, with its kind.
  attachPasted: (pictures) => ipcRenderer.invoke('columbina:attach-pasted', Array.from(
    Array.isArray(pictures) ? pictures : [],
    (picture) => ({ type: picture?.type, bytes: picture?.bytes }),
  )),
  schedules: (asked) => ipcRenderer.invoke('columbina:schedules', {
    what: asked?.what, id: asked?.id, task: asked?.task === undefined ? undefined : { ...asked.task },
  }),
  worktrees: (asked) => ipcRenderer.invoke('columbina:worktrees', {
    what: asked?.what, cwd: asked?.cwd, name: asked?.name, force: asked?.force === true,
  }),
  qqGroups: (asked) => ipcRenderer.invoke('columbina:qq-groups', { what: asked?.what, id: asked?.id, name: asked?.name }),
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
  feishuStart: (asked) => ipcRenderer.invoke('columbina:feishu-start', { language: asked?.language }),
  feishuStop: () => ipcRenderer.invoke('columbina:feishu-stop'),
  feishuForget: () => ipcRenderer.invoke('columbina:feishu-forget'),
  feishuLog: () => ipcRenderer.invoke('columbina:feishu-log'),
  feishuWritable: (writable) => ipcRenderer.invoke('columbina:feishu-writable', writable === true),
  openFeishuFolder: () => ipcRenderer.invoke('columbina:feishu-folder'),
  setUpFeishu: (asked) => ipcRenderer.invoke('columbina:set-up-feishu', { language: asked?.language }),
  onFeishu: (callback) => subscribe('columbina:feishu', callback),
  feishuThinking: (asked) => ipcRenderer.invoke('columbina:feishu-thinking', { model: asked?.model ?? null, effort: asked?.effort ?? null }),
  wecomStatus: () => ipcRenderer.invoke('columbina:wecom-status'),
  wecomStart: (asked) => ipcRenderer.invoke('columbina:wecom-start', { language: asked?.language }),
  wecomStop: () => ipcRenderer.invoke('columbina:wecom-stop'),
  wecomForget: () => ipcRenderer.invoke('columbina:wecom-forget'),
  wecomLog: () => ipcRenderer.invoke('columbina:wecom-log'),
  wecomWritable: (writable) => ipcRenderer.invoke('columbina:wecom-writable', writable === true),
  openWecomFolder: () => ipcRenderer.invoke('columbina:wecom-folder'),
  setUpWecom: (asked) => ipcRenderer.invoke('columbina:set-up-wecom', { language: asked?.language }),
  onWecom: (callback) => subscribe('columbina:wecom', callback),
  wecomThinking: (asked) => ipcRenderer.invoke('columbina:wecom-thinking', { model: asked?.model ?? null, effort: asked?.effort ?? null }),
  qqStatus: () => ipcRenderer.invoke('columbina:qq-status'),
  qqStart: (asked) => ipcRenderer.invoke('columbina:qq-start', { language: asked?.language }),
  qqStop: () => ipcRenderer.invoke('columbina:qq-stop'),
  qqForget: () => ipcRenderer.invoke('columbina:qq-forget'),
  qqLog: () => ipcRenderer.invoke('columbina:qq-log'),
  openQqFolder: () => ipcRenderer.invoke('columbina:qq-folder'),
  setUpQq: (asked) => ipcRenderer.invoke('columbina:set-up-qq', { language: asked?.language }),
  onQq: (callback) => subscribe('columbina:qq', callback),
  qqThinking: (asked) => ipcRenderer.invoke('columbina:qq-thinking', { model: asked?.model ?? null, effort: asked?.effort ?? null }),
  pets: () => ipcRenderer.invoke('columbina:pets'),
  setPet: (asked) => ipcRenderer.invoke('columbina:set-pet', { on: asked?.on, chosen: asked?.chosen }),
  openPetsFolder: () => ipcRenderer.invoke('columbina:open-pets-folder'),
  onPet: (callback) => subscribe('columbina:pet', callback),
});
