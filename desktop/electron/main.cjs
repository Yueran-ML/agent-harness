const { app, BrowserWindow, Menu, Notification, Tray, dialog, ipcMain, nativeImage, screen, session, shell } = require('electron');
const { spawn } = require('node:child_process');
const { createInterface } = require('node:readline');
const {
  constants, createWriteStream, existsSync, mkdirSync, readFileSync, realpathSync, renameSync, statSync, writeFileSync,
} = require('node:fs');
const { copyFile, mkdir, open, readdir, realpath, stat, writeFile } = require('node:fs/promises');
const path = require('node:path');
const { StringDecoder } = require('node:string_decoder');
const { pathToFileURL, fileURLToPath } = require('node:url');
const pets = require('./pets.cjs');
const engine = require('./engine.cjs');

let window;
let service = null;
let stopping = null;
let exiting = false;
let serviceOperation = Promise.resolve();
let pendingServiceOperations = 0;
let closeTimer;
const approvedDirectories = new Set();
const indexPath = path.resolve(__dirname, '../dist/index.html');
const indexUrl = pathToFileURL(indexPath).href;
// The page of the window that takes a key. It shows nothing that a model wrote.
const keyPath = path.resolve(__dirname, '../dist/key.html');
const keyUrl = pathToFileURL(keyPath).href;
// The page of the window of the pet, and the pets that come with the application.
const petPath = path.resolve(__dirname, '../dist/pet.html');
const petUrl = pathToFileURL(petPath).href;
const petsBuiltIn = path.resolve(__dirname, '../dist/pets');
// The page of the floating window of the video scopes of the Color page (T19): it reaches the
// engine for its own scopes alone.
const scopesPath = path.resolve(__dirname, '../dist/scopes.html');
const scopesUrl = pathToFileURL(scopesPath).href;
/** What the window of the scopes may call of the engine: its own scopes. */
const SCOPE_WINDOW_METHODS = new Set(['scopes.subscribe', 'scopes.set', 'scopes.unsubscribe']);
// The window of the pet while it shows, what it was told last of the task, how it keeps looking
// at the task, and where a drag of the pet began: the pointer and the window.
let petWindow = null;
let petShown = { state: 'idle', label: '', bubble: '' };
let petLooking = null;
let petDrag = null;
/** How often the pet looks at what the service process wrote of its task, in milliseconds. */
const PET_LOOK_MS = 1000;
/** How large the window of the pet is: room for the pet, and for its bubble above it. */
const PET_SIZE = { width: 220, height: 240 };
/**
 * An assistant in the chat of another platform that the application runs: a command of the program,
 * set up in a window of its own that shows nothing that a model wrote. It has the name of the
 * platform; what is set up there, as the window says it; the fields of the document that the window
 * hands to the program; the pages that the window may open, which are the consoles where that is
 * made, and what they are; and whether its owner may let its agent change the files of its folder.
 *
 * Of each, the application runs one at most, and keeps the stopping of it while that lasts. Another
 * program may run one, such as a terminal: the program allows one of each at a time on a computer,
 * and says whether one runs. The window is kept while it is open, with the language that it speaks,
 * whether something was stored from it, and the promise that says so when it has closed. Whether
 * something is being stored is kept too, as the window is not closed then. So is how the last
 * assistant that the application ran ended where it did not end well, or why it did not start.
 */
function assistantOf(kind, name, what, fields, pages, where, writable = true) {
  const page = path.resolve(__dirname, `../dist/${kind}.html`);
  return {
    kind, name, what, fields, pages, where, writable, page, url: pathToFileURL(page).href,
    child: null, stopping: null, storing: false, ended: null,
    window: null, asked: { language: 'en' }, changed: false, closed: null,
  };
}
const assistants = {
  feishu: assistantOf('feishu', 'Feishu', 'the app of Feishu', ['domain', 'app_id', 'app_secret'],
    ['https://open.feishu.cn/app', 'https://open.larksuite.com/app'], 'the console of Feishu or Lark'),
  wecom: assistantOf('wecom', 'WeCom', 'the bot of WeCom', ['bot_id', 'secret'],
    ['https://work.weixin.qq.com/wework_admin/frame'], 'the admin console of WeCom'),
  // Its agents change no file, whatever the user says.
  qq: assistantOf('qq', 'QQ', 'the bot of QQ', ['app_id', 'secret'], ['https://q.qq.com/'],
    'the open platform of QQ', false),
};
// The bar of the window in each theme: the furthest surface of `src/ui/tokens.css`, and its text.
const titleBarThemes = {
  moonveil: { color: '#17161a', symbolColor: '#eceae4', height: 40 },
  moonwhite: { color: '#f5f4f0', symbolColor: '#1b1c24', height: 40 },
};

// How much is kept of what a service wrote for errors: so many lines, and of a line so many characters.
const TAIL_LINES = 30;
const TAIL_LINE_CHARS = 400;
// How many folders are kept as recent projects.
const RECENT_MOST = 8;
// How long `columbina setup` may take. It waits up to 30 seconds for a keyring that is locked.
const SETUP_MS = 45_000;
// How much of what it writes is read, in characters. Its answer is one line.
const SETUP_OUTPUT_MOST = 512 * 1024;
const NO_SETUP = 'No service program is set up for this application, so its services cannot be set up from here.';
// How much of what `columbina login --status` writes is read: its line may hold the picture of the user.
const ACCOUNT_OUTPUT_MOST = 2 * 1024 * 1024;
// How long the program that signs in to GitHub is given to end after its input was closed.
const SIGN_IN_STOP_MS = 3_000;
// How long an assistant is given to end after its input was closed. It takes a few seconds to stop
// showing that it writes an answer, or to say so in the answers that it writes. Where it has not
// ended then, it is ended, and given a few seconds more to be gone; otherwise it is said not to have stopped.
const ASSISTANT_STOP_MS = 6_000;
const ASSISTANT_KILL_MS = 5_000;
// How long a line may be that an assistant writes of how it is doing. Its lines are short.
const ASSISTANT_LINE_MOST = 64 * 1024;
// The languages in which an assistant can say things by itself, such as its help, and the one for
// the language of the page that starts it. For a page in another language, the assistant goes by
// what its platform speaks.
const ASSISTANT_LANGUAGES = ['zh', 'en'];
// How hard the model of an assistant may be asked to think.
const EFFORTS = ['default', 'off', 'low', 'high', 'max'];
const assistantLanguageOf = (language) => (language === 'zh-CN' ? 'zh' : language === 'en' ? 'en' : null);
// How long the login shell of the user may take on macOS to say what it has, and how much of what it writes is read.
const LOGIN_SHELL_MS = 5_000;
const LOGIN_SHELL_OUTPUT_MOST = 64 * 1024;
// What the PATH of an application opened from the Finder leaves out on macOS: the programs of Homebrew, and /usr/local.
const MAC_PROGRAMS = ['/opt/homebrew/bin', '/opt/homebrew/sbin', '/usr/local/bin', '/usr/local/sbin'];
// The variables of the proxy, each in the two cases that programs read it in. The login shell is
// asked for them with the PATH: a user who sets a proxy for a terminal sets it there.
const SHELL_PROXIES = [['HTTPS_PROXY', 'https_proxy'], ['HTTP_PROXY', 'http_proxy'], ['ALL_PROXY', 'all_proxy'], ['NO_PROXY', 'no_proxy']];
const SHELL_VARIABLES = ['PATH', ...SHELL_PROXIES.flat()];

/**
 * How the bar of the window is drawn. The page draws the bar itself and leaves room for the
 * buttons of the system: on Windows and Linux they lie over its right end, in the colours of
 * the theme, and on macOS they stand at its left end, in the middle of its height.
 */
function windowBar(theme) {
  if (process.platform === 'darwin') return { titleBarStyle: 'hidden', trafficLightPosition: { x: 16, y: 13 } };
  return { titleBarStyle: 'hidden', titleBarOverlay: theme };
}
// What the shell says by itself, in every language that the interface speaks, each under the
// name that a page gives it. Everything else that is said is said by a page.
const WORDS = {
  en: { chooseFolder: 'Choose a project folder', chooseFiles: 'Choose files to attach', open: 'Open Columbina', quit: 'Quit Columbina', hidePet: 'Hide the pet',
    petWaiting: (tool) => (tool ? `Waiting for your yes to ${tool}.` : 'Waiting for your yes.'), petDone: 'Done.', petFailed: 'Something went wrong.',
    chooseExport: 'Export timeline', chooseMedia: 'Choose media to import', media: 'Video, audio and pictures', allFiles: 'All files',
    importStills: 'Import Stills', exportStills: 'Export Stills', stills: 'Still images', generateLut: 'Generate LUT', cube: 'Cube LUT' },
  ja: { chooseFolder: 'プロジェクトフォルダーを選択', chooseFiles: '添付するファイルを選択', open: 'Columbina を開く', quit: 'Columbina を終了', hidePet: 'ペットをしまう',
    petWaiting: (tool) => (tool ? `${tool} の承認を待っています。` : '承認を待っています。'), petDone: '終わりました。', petFailed: 'うまくいきませんでした。',
    chooseExport: 'タイムラインを書き出す', chooseMedia: '読み込むメディアを選択', media: '動画・音声・画像', allFiles: 'すべてのファイル',
    importStills: 'スチルを読み込む', exportStills: 'スチルを書き出す', stills: '静止画', generateLut: 'LUT を生成', cube: 'Cube LUT' },
  'zh-CN': { chooseFolder: '选择项目文件夹', chooseFiles: '选择要附上的文件', open: '打开 Columbina', quit: '退出 Columbina', hidePet: '收起桌宠',
    petWaiting: (tool) => (tool ? `等你点头：${tool}。` : '等你点头。'), petDone: '做完了。', petFailed: '遇到点难题。',
    chooseExport: '导出时间线', chooseMedia: '选择要导入的媒体', media: '视频、音频和图片', allFiles: '所有文件',
    importStills: '导入静帧', exportStills: '导出静帧', stills: '静帧图像', generateLut: '生成 LUT', cube: 'Cube LUT' },
};
/**
 * The methods of the engine that the page may call. The view is attached and detached by the
 * shell alone, which holds the handle of the window. Media is imported from files that the user
 * chose in the dialog of the system; a transaction of the page may name no file (see
 * `transactionOfThePage`), and undo and redo go through the history of the engine.
 */
const PAGE_METHODS = new Set([
  'project.get', 'transaction.apply', 'history.undo', 'history.redo', 'media.make_proxy',
  // The LUTs of the folders that a node's LUT is looked for in (names only, nothing outside them),
  // and Display HDR On Viewers If Available, which the page sends again at each connect.
  'luts.list', 'view.display', 'view.viewport',
  'transport.load', 'transport.play', 'transport.pause', 'transport.seek', 'transport.step', 'transport.state', 'transport.proxies',
  'view.place', 'view.show', 'timeline.duration', 'media.thumbnail', 'media.waveform', 'render.cancel',
  // The figures of the frame shown, which the Primaries palette's Auto Color reads.
  'scopes.statistics',
  // The scopes of the Color page, each a subscription of this connection (see `keepScopes`).
  'scopes.subscribe', 'scopes.set', 'scopes.unsubscribe',
  // The colour of a point of the picture, and the pointer on a view, for the pickers of the Color
  // page, the line of a wipe and the windows.
  'view.sample', 'view.pointer', 'window.bind', 'window.input', 'window.unbind',
  // The Gallery of the Color page and the comparisons of its viewer, which name stills and albums
  // by their ids only: a wipe names no file (see `comparisonOfThePage`), and stills are imported
  // from files chosen in the dialog and exported where Save As says (`engine-still-import`, `-export`).
  'viewer.wipe', 'viewer.split', 'still.grab', 'still.get', 'still.apply', 'still.label', 'still.delete', 'still.move',
  'gallery.list', 'gallery.thumbnail', 'gallery.album.create', 'gallery.album.rename', 'gallery.album.delete', 'grade.append_node',
  // The Highlight modes and Bypass Color of the Color page's viewer, and its thumbnails as they are
  // graded. Generate LUT names a file, which Save As chooses (`engine-export-lut`).
  'viewer.highlight', 'viewer.bypass', 'timeline.thumbnail',
  // Window presets, a library of this user's apart from every project (RM20 pp. 3198–3199).
  'window_presets.list', 'window_presets.save', 'window_presets.update', 'window_presets.delete', 'window_presets.apply',
  // The LUT browser's thumbnails and its Live Preview on the viewer, of LUTs named in the folders of
  // LUTs alone (the engine refuses any other name), and the Color Viewer LUT on the Color page alone.
  'luts.preview', 'viewer.preview', 'viewer.lut',
  // Window tracking (RM20 pp. 3231–3238): a track of the selected window, its stop, and the edits
  // of its track data, which name the window by its grade, node and id alone.
  'track.start', 'track.stop', 'track.edit',
  // The meters, the loudness meter and the analyses of sound (phase 3, T9b), which name timelines,
  // clips, tracks and buses by their ids alone.
  'audio.meters', 'audio.loudness', 'audio.analyze', 'audio.normalize', 'audio.level_match', 'audio.cancel',
]);
/** The points on an edge of the cubes that Generate LUT makes (Reference Manual 20, p. 3413). */
const LUT_SIZES = [17, 33, 65];
/** The endings of the pictures that the engine makes stills of (PNG, TIFF, OpenEXR; not JPEG, which is Y'CbCr). */
const STILL_ENDINGS = ['png', 'tif', 'tiff', 'exr'];
/** The endings of the files that the engine reads as media, offered first in the dialog. */
const MEDIA_ENDINGS = [
  'mp4', 'm4v', 'mov', 'mkv', 'webm', 'avi', 'ts', 'm2ts', 'mts', 'mpg', 'mpeg', 'vob',
  'wav', 'mp3', 'flac', 'ogg', 'oga', 'ogv', 'opus', 'aac', 'm4a', 'png', 'jpg', 'jpeg', 'webp',
];
/** Where the studio keeps its project in a project folder. */
const STUDIO_FOLDER = ['.columbina', 'studio'];
/** How long the migration of the old projects of a folder may take, and how much of what it writes is read. */
const MIGRATE_MS = 10 * 60_000;
const MIGRATE_OUTPUT_MOST = 1024 * 1024;
/** How many old projects the page is told of, how many lines of each, and how long a line is. */
const MIGRATIONS_TOLD_MOST = 200;
const MIGRATION_LINES_MOST = 50;
const MIGRATION_LINE_MOST = 2000;
/**
 * The folders whose old projects were looked at while the application runs, by their key: `running`,
 * `done` where nothing was left to do, `again` where something failed and is tried the next time.
 */
const migrations = new Map();
/** How far from the corner of the window a viewer may lie, and how large it may be, in pixels of the screen. */
const VIEW_PLACE_MOST = 32_768;
const VIEW_SIZE_MOST = 16_384;
// What the start at login gives the application, which then waits in the tray with its window put away.
const HIDDEN = '--hidden';
// The project that the service of the moment works in: the process, its folder, and whether it is a replay.
let serviceProject = null;
// The files that the user chose or dropped to go with a message, by the ids that the page knows
// them by. The page is told their names and sizes, and never where they lie.
const attachments = new Map();
let attachmentNumber = 0;
/** How large a file that goes with a message may be, and how many go with one message. */
const ATTACHMENT_MOST_BYTES = 25 * 1024 * 1024;
const ATTACHMENTS_MOST = 10;
/** How many chosen files the shell holds at most, for all the messages that are being written. */
const ATTACHMENTS_HELD_MOST = 200;
/** Where the files that go with messages are kept in a project, from its folder. */
const ATTACHMENTS_FOLDER = ['.columbina', 'attachments'];
/** The kinds of picture that may be pasted, with the ending of the file that each is written to. */
const PASTED_KINDS = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp' };
let pastedNumber = 0;
// Where a package asks which version of the application is the newest: the releases of its
// repository on GitHub. The page of a release is named here as well, so that the page is never
// told an address of GitHub's answer to open.
const RELEASES = 'https://api.github.com/repos/Yueran-ML/agent-harness/releases?per_page=30';
const RELEASE_PAGE = 'https://github.com/Yueran-ML/agent-harness/releases/tag/desktop-v';
/** How long GitHub is given to answer, and how long an answer is taken as it is before GitHub is asked again. */
const UPDATE_MS = 15_000;
const UPDATE_KEPT_MS = 60 * 60_000;
/** How much of the answer is read, in characters. A list of thirty releases is far less. */
const UPDATE_OUTPUT_MOST = 4 * 1024 * 1024;
// What GitHub said last, and when, and the question that is under way.
let updateKnown = null;
let updateAsking = null;
// The session that asks GitHub: one of its own, kept in memory only. The session of the window
// lets nothing out but the files of the application, and stays so.
const RELEASES_PARTITION = 'columbina-releases';
let releasesSession = null;
/** The language that a page names, if the shell speaks it, and English if not. */
const spoken = (language) => (typeof language === 'string' && Object.hasOwn(WORDS, language) ? language : 'en');
// The window for the key while it is open, what it was opened for, whether something was
// stored or deleted from it, and the promise that says so when it has closed.
let keyWindow = null;
// The floating window of the video scopes while it is open, and what the page said last of what it
// measures: whether the timeline viewer has something loaded, and the language it speaks in.
let scopesWindow = null;
let scopesAsked = { loaded: false, language: 'en' };
// Counted each time the window of the scopes lets its scopes go, so that a scope it asked for
// before is let go as it is answered (see `columbina-scopes:call`).
let windowScopesRound = 0;
let keyAsked = { service: null, language: 'en' };
let keyChanged = false;
let keyClosed = null;
// The services that the window for the key was told of last, and the pages on which their keys are made.
let keyListed = { names: [], pages: [] };
// The services that the main window was told of last, by name. A model is listed and kept for one of them.
let mainListed = [];
// What the service of the moment wrote last for errors. It is told of when the service has ended.
let lastWords = { lines: [], rest: '' };
// What is kept from one start to the next. Read when the application is ready.
let preferences = {
  theme: 'moonveil', recent: [], githubSkipped: false, feishuOn: false, feishuLanguage: null,
  wecomOn: false, wecomLanguage: null, qqOn: false, qqLanguage: null, language: null,
  petOn: false, pet: 'builtin/moon', petAt: null,
};
// The icon in the tray, once the window was put away there, and whether the user asked from there to quit.
let tray = null;
let quitting = false;
// Whether the window was asked for before it was there: by a second start while the first one began.
let wanted = false;
let flashing = false;
// The notification that shows, held so that it is not cleared away while it does.
let shown = null;
// On macOS, what the login shell of the user said of its variables, and the asking for it while it lasts.
let loginShell = null;
let askingLoginShell = null;
// The program that signs in to GitHub while it waits for the user. There is one at most.
let signingIn = null;
// The engine of the studio while a project is open in it: its number, which every event to the
// page carries, the directory of its project, and the session with it. Opened and closed one
// after the other.
let studio = null;
let studioNumber = 0;
let studioOperation = Promise.resolve();
// The viewer of the studio: where the page keeps it, in pixels of the screen from the corner of
// the client area; whether the page wants it shown; whether it is blank, hidden because the page
// is being loaded again until it says where the viewer is; whether the engine shows it now; how
// the view of the engine stands there: `none`, `attaching`, `attached`, `unsupported` (macOS, or
// an engine without a window) or `failed`; the number of the view that was made last, and what
// the page was told of it last.
const MONITORS = ['source', 'timeline'];
const viewers = Object.fromEntries(MONITORS.map(monitor => [monitor, { rect: null, shown: true, blank: false, sent: true, state: 'none', generation: 0, told: null }]));

app.setName('Columbina');
if (process.platform === 'win32') app.setAppUserModelId('dev.columbina.desktop');

const logPath = () => path.join(app.getPath('logs'), 'columbina-service.log');
const engineLogPath = () => path.join(app.getPath('logs'), 'columbina-engine.log');
const assistantLogPath = (one) => path.join(app.getPath('logs'), `columbina-${one.kind}.log`);
/** Whether the application runs an assistant: in Feishu, in WeCom, or in both. */
const anyAssistant = () => Object.values(assistants).some((one) => one.child !== null);
const preferencesPath = () => path.join(app.getPath('userData'), 'preferences.json');

/** Text as it may be shown: on one line, without what steers a terminal. */
function plain(text) {
  return text
    .replace(/\u001b\[[0-9;?]*[ -/]*[@-~]/g, '')
    .replace(/[\u0000-\u001f\u007f]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}
/** The beginning of a text that is longer than it may be, marked as cut. */
function short(text, most) {
  const characters = Array.from(text);
  return characters.length > most ? `${characters.slice(0, most).join('')}…` : text;
}
/** Keep the last lines that a service writes for errors, and return where they are kept. */
function listenForErrors(child) {
  const words = { lines: [], rest: '' };
  const decoder = new StringDecoder('utf8');
  child.stderr.on('data', (chunk) => {
    // What ends a line on Windows beside the line feed is taken out with whatever else steers a terminal.
    const parts = (words.rest + decoder.write(chunk)).split('\n');
    // A line without an end is not held without bound either.
    words.rest = parts.pop().slice(0, 4 * TAIL_LINE_CHARS);
    for (const part of parts) {
      const line = plain(part);
      if (line) words.lines.push(short(line, TAIL_LINE_CHARS));
    }
    if (words.lines.length > TAIL_LINES) words.lines.splice(0, words.lines.length - TAIL_LINES);
  });
  return words;
}
/** What was kept at the last start, as far as it can be used. Anything else is left out. */
function readPreferences() {
  let kept = null;
  try { kept = JSON.parse(readFileSync(preferencesPath(), 'utf8')); } catch { /* Nothing was kept, or it cannot be read. */ }
  // Whatever the file holds, only a theme that there is and folders that can be folders are taken from it.
  const given = kept ?? {};
  const theme = typeof given.theme === 'string' && Object.hasOwn(titleBarThemes, given.theme) ? given.theme : 'moonveil';
  const recent = [];
  for (const folder of Array.isArray(given.recent) ? given.recent : []) {
    if (typeof folder !== 'string' || !path.isAbsolute(folder)) continue;
    if (recent.some((known) => directoryKey(known) === directoryKey(folder))) continue;
    recent.push(folder);
    if (recent.length === RECENT_MOST) break;
  }
  // Whether the user said on the first page that GitHub can wait, whether each assistant ran when
  // the user last started or stopped it, in which language it said things by itself, and the
  // language of the page, in which the menu of the tray speaks before the page has said it.
  const known = (language) => (ASSISTANT_LANGUAGES.includes(language) ? language : null);
  return {
    theme, recent, githubSkipped: given.githubSkipped === true,
    feishuOn: given.feishuOn === true, feishuLanguage: known(given.feishuLanguage),
    wecomOn: given.wecomOn === true, wecomLanguage: known(given.wecomLanguage),
    qqOn: given.qqOn === true, qqLanguage: known(given.qqLanguage),
    language: typeof given.language === 'string' && Object.hasOwn(WORDS, given.language) ? given.language : null,
    // Whether the pet shows, which one, and where it stood.
    petOn: given.petOn === true,
    pet: typeof given.pet === 'string' && /^(builtin|user)\/[^\\/]{1,128}$/.test(given.pet) ? given.pet : 'builtin/moon',
    petAt: Number.isInteger(given.petAt?.x) && Number.isInteger(given.petAt?.y) ? { x: given.petAt.x, y: given.petAt.y } : null,
  };
}
/** Write down what is kept. It is written beside its place and moved there, so that a start never finds half of it. */
function keepPreferences() {
  const file = preferencesPath();
  try {
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(`${file}.tmp`, `${JSON.stringify(preferences, null, 2)}\n`, 'utf8');
    renameSync(`${file}.tmp`, file);
  } catch { /* What cannot be kept is chosen again at the next start. */ }
}
/** Put a folder that the user chose first among the recent projects. */
function remember(cwd) {
  const others = preferences.recent.filter((known) => directoryKey(known) !== directoryKey(cwd));
  preferences.recent = [cwd, ...others].slice(0, RECENT_MOST);
  keepPreferences();
}
function isFolder(folder) {
  try { return statSync(folder).isDirectory(); } catch { return false; }
}

function checkSender(event) {
  if (!window || event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame || event.senderFrame.url !== indexUrl) {
    throw new Error('IPC sender is not the application window.');
  }
}
function checkScopesSender(event) {
  if (!scopesWindow || event.sender !== scopesWindow.webContents || event.senderFrame !== scopesWindow.webContents.mainFrame
    || event.senderFrame.url !== scopesUrl) {
    throw new Error('IPC sender is not the window of the scopes.');
  }
}
function checkKeySender(event) {
  if (!keyWindow || event.sender !== keyWindow.webContents || event.senderFrame !== keyWindow.webContents.mainFrame || event.senderFrame.url !== keyUrl) {
    throw new Error('IPC sender is not the window for the key.');
  }
}
function checkAssistantSender(event, one) {
  const opened = one.window;
  if (!opened || event.sender !== opened.webContents || event.senderFrame !== opened.webContents.mainFrame || event.senderFrame.url !== one.url) {
    throw new Error(`IPC sender is not the window for ${one.what}.`);
  }
}
function checkPetSender(event) {
  if (!petWindow || event.sender !== petWindow.webContents || event.senderFrame !== petWindow.webContents.mainFrame || event.senderFrame.url !== petUrl) {
    throw new Error('IPC sender is not the window of the pet.');
  }
}
/** A window opens no other window, goes to no other page and holds no page inside its own. */
function closeToTheWorld(opened) {
  opened.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  opened.webContents.on('will-navigate', (event) => event.preventDefault());
  opened.webContents.on('will-attach-webview', (event) => event.preventDefault());
}
function tell(channel, value) {
  if (window && !window.isDestroyed() && !window.webContents.isDestroyed()) window.webContents.send(channel, value);
}
function directoryKey(directory) {
  return process.platform === 'win32' ? directory.toLowerCase() : directory;
}
function approvedDirectory(options, purpose = 'starting the service') {
  if (typeof options?.cwd !== 'string' || !options.cwd || !path.isAbsolute(options.cwd)) {
    throw new Error(`Choose a project folder before ${purpose}.`);
  }
  const cwd = realpathSync(options.cwd);
  if (!approvedDirectories.has(directoryKey(cwd))) {
    throw new Error('The working directory must be selected in the native folder dialog.');
  }
  if (!statSync(cwd).isDirectory()) throw new Error('The working directory is not a folder.');
  return cwd;
}
function enqueueService(operation) {
  pendingServiceOperations += 1;
  const result = serviceOperation.then(operation);
  serviceOperation = result.catch(() => {}).finally(() => { pendingServiceOperations -= 1; });
  return result;
}
function killTree(child) {
  if (!child.pid) return;
  if (process.platform === 'win32') {
    const killer = spawn('taskkill.exe', ['/pid', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
    killer.on('error', () => { try { child.kill(); } catch {} });
  } else {
    try { process.kill(-child.pid, 'SIGKILL'); } catch { try { child.kill('SIGKILL'); } catch {} }
  }
}
async function stopService() {
  if (stopping) return stopping;
  const child = service;
  if (!child) return;
  stopping = new Promise((resolve) => {
    let done = false;
    let timer;
    const finish = () => {
      if (done) return;
      done = true; clearTimeout(timer);
      if (service === child) service = null;
      resolve();
    };
    child.once('close', finish);
    // EOF is the protocol's graceful shutdown fallback if the renderer has gone away.
    child.stdin.end();
    timer = setTimeout(() => { killTree(child); setTimeout(finish, 500); }, 2500);
    if (child.exitCode !== null || child.signalCode !== null) finish();
  });
  try { await stopping; } finally { stopping = null; }
}
/**
 * The folder of a conversation that belongs to no project. It lies beside the home of the program,
 * as the folder of the assistant in Feishu does: ~/Columbina/chat where the home is ~/.columbina.
 */
function chatFolder() {
  const home = process.env.COLUMBINA_HOME || path.join(app.getPath('home'), '.columbina');
  return path.join(path.dirname(home), 'Columbina', 'chat');
}
/**
 * Start the service process once the one that runs has stopped. `where` names the folder that it
 * works in when the turn of this start has come, and `words` follow `serve` in the command of the
 * program, such as a profile. A replay takes none.
 */
async function startServiceIn(where, words) {
  if (exiting) throw new Error('The application is closing.');
  const cwd = where();
  await stopService();
  if (askingLoginShell) await askingLoginShell;
  if (exiting) throw new Error('The application is closing.');
  const config = serviceCommand();
  const args = config.mode === 'service' && config.args.includes('serve') ? [...config.args, ...words] : config.args;
  mkdirSync(path.dirname(logPath()), { recursive: true });
  const log = createWriteStream(logPath(), { flags: 'a' });
  log.on('error', () => {});
  const child = spawn(config.executable, args, {
    // The service writes what its task does to the file that the pet reads, as the plugin
    // columbina.pets does where it is named. See docs/pets.md.
    // Its agent reaches the studio's project through the same engine as the window.
    cwd, env: { ...config.env, COLUMBINA_PET_STATE: petStateFile(), ...engineOfTheService() }, windowsHide: true,
    detached: process.platform !== 'win32', stdio: ['pipe', 'pipe', 'pipe'],
  });
  service = child;
  serviceProject = { child, cwd, mode: config.mode };
  lastWords = listenForErrors(child);
  const lines = createInterface({ input: child.stdout, crlfDelay: Infinity });
  lines.on('line', (line) => { if (service === child) tell('columbina:line', line); });
  child.stderr.pipe(log);
  child.stdin.on('error', (error) => {
    if (service === child) tell('columbina:closed', `The service input closed: ${error.message}`);
  });
  child.on('error', (error) => {
    if (service === child) tell('columbina:closed', `The service could not start: ${error.message}`);
  });
  child.on('close', (code, signal) => {
    lines.close();
    log.end();
    if (service === child) {
      service = null;
      tell('columbina:closed', code === 0 ? 'The service has shut down.' : `The service process exited (${signal || code}).`);
    }
  });
  await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); });
  return { mode: config.mode, cwd };
}
/**
 * Ask the login shell of the user, the way a terminal starts it, what its PATH is and which proxy
 * it sets. The answer holds what it said of each variable, which is empty for one that it does not
 * set, and is null where it says nothing in time.
 *
 * An application that is opened from the Finder or the Dock on macOS is given only
 * `/usr/bin:/bin:/usr/sbin:/sbin` and no proxy. The commands that an agent runs would not find
 * what the user has installed, such as Homebrew, Node or Docker, nor reach what the user reaches
 * through the proxy. Each variable is marked at both ends by its own name, so that what the files
 * of the shell print by themselves is left out.
 */
function askLoginShell() {
  const program = process.env.SHELL && path.isAbsolute(process.env.SHELL) ? process.env.SHELL : '/bin/zsh';
  const markOf = (name) => `_COLUMBINA_${name}_`;
  const script = SHELL_VARIABLES.map((name) => `printf '${markOf(name)}%s${markOf(name)}' "$${name}"`).join('; ');
  return new Promise((resolve) => {
    let child = null;
    let written = '';
    const timer = setTimeout(() => {
      try { child?.kill(); } catch { /* It has ended. */ }
      resolve(null);
    }, LOGIN_SHELL_MS);
    const answer = (found) => { clearTimeout(timer); resolve(found); };
    try {
      child = spawn(program, ['-ilc', script], {
        // Oh My Zsh would ask first whether it may update itself.
        env: { ...process.env, DISABLE_AUTO_UPDATE: 'true' }, stdio: ['ignore', 'pipe', 'ignore'],
      });
    } catch {
      answer(null);
      return;
    }
    child.stdout.on('data', (chunk) => { if (written.length < LOGIN_SHELL_OUTPUT_MOST) written += chunk; });
    child.on('error', () => answer(null));
    child.on('close', () => {
      const said = {};
      for (const name of SHELL_VARIABLES) {
        const parts = written.split(markOf(name));
        if (parts.length >= 3) said[name] = parts[1].trim();
      }
      answer(said);
    });
  });
}
/** The PATH of the commands of the service on macOS: the one of the login shell, or the usual places added to the one there is. */
function macPath() {
  if (loginShell?.PATH) return loginShell.PATH;
  const places = (process.env.PATH ?? '').split(':').filter(Boolean);
  return [...places, ...MAC_PROGRAMS.filter((place) => !places.includes(place))].join(':');
}
/**
 * The Python that a package brings, with Columbina in it, or null in development.
 * It lies in `resources/python`, the way the packaging puts it there.
 */
function bundledPython() {
  if (!app.isPackaged) return null;
  const root = path.join(process.resourcesPath, 'python');
  return process.platform === 'win32' ? path.join(root, 'python.exe') : path.join(root, 'bin', 'python3');
}
/**
 * The environment of the service of a package: that of the application, with the PATH and the
 * proxy of the user on macOS. Nothing in it is meant for the Python of the package, since the
 * commands of the agent inherit it: that Python keeps to itself through the flags that it is
 * started with.
 */
function bundledEnvironment() {
  const env = { ...process.env };
  if (process.platform === 'darwin') {
    env.PATH = macPath();
    // A proxy that the application was given goes first, in whichever case it is written.
    for (const names of SHELL_PROXIES) {
      if (names.some((name) => env[name])) continue;
      for (const name of names) if (loginShell?.[name]) env[name] = loginShell[name];
    }
  }
  return env;
}
/**
 * The program of the service: the one that COLUMBINA_SERVICE_EXECUTABLE names, else the Python
 * of the package, else, in development only, the replay. A package never replays: what it shows
 * is a real service, or that there is none.
 */
function serviceCommand() {
  const executable = process.env.COLUMBINA_SERVICE_EXECUTABLE;
  if (!executable) {
    const python = bundledPython();
    if (python === null) return { executable: process.execPath, args: [path.join(__dirname, 'mock-service.mjs')], mode: 'mock', env: { ...process.env, ELECTRON_RUN_AS_NODE: '1' } };
    if (!existsSync(python)) throw new Error('This package has no service of its own. Download it again from the releases of Columbina.');
    // -I reads no PYTHON variable and no site of the user, and puts no folder of the project before
    // the packages of the package. -B writes nothing into the package, whose files are all compiled.
    return { executable: python, args: ['-I', '-B', '-m', 'columbina', 'serve'], mode: 'service', env: bundledEnvironment() };
  }
  if (!path.isAbsolute(executable)) throw new Error('COLUMBINA_SERVICE_EXECUTABLE must be an absolute executable path.');
  const args = process.env.COLUMBINA_SERVICE_ARGS ? JSON.parse(process.env.COLUMBINA_SERVICE_ARGS) : ['-m', 'columbina', 'serve'];
  if (!Array.isArray(args) || args.some((value) => typeof value !== 'string')) throw new Error('COLUMBINA_SERVICE_ARGS must be a JSON array of strings.');
  return { executable, args, mode: 'service', env: { ...process.env } };
}
/**
 * The program of the engine of the studio, or null where there is none: in a package the one
 * that it brings in `resources/engine`; in development the one that COLUMBINA_ENGINE names by
 * its whole path, else the newest build of `engine/` of the repository.
 */
function engineProgram() {
  const name = process.platform === 'win32' ? 'columbina-engine.exe' : 'columbina-engine';
  if (app.isPackaged) {
    const brought = path.join(process.resourcesPath, 'engine', name);
    return existsSync(brought) ? brought : null;
  }
  const named = process.env.COLUMBINA_ENGINE;
  if (named) return path.isAbsolute(named) ? named : null;
  const built = ['release', 'debug']
    .map((kind) => path.resolve(__dirname, '../../engine/target', kind, name))
    .filter((file) => existsSync(file))
    .map((file) => {
      let at = 0;
      try { at = statSync(file).mtimeMs ?? 0; } catch { /* It is gone. */ }
      return { file, at };
    });
  // The newest; of two as new, the release.
  built.sort((one, other) => other.at - one.at);
  return built[0]?.file ?? null;
}
/** The folder of the libraries of FFmpeg that the engine reads media with, where it is not beside it in a package. */
function engineLibraries(program) {
  if (app.isPackaged) return path.dirname(program);
  return process.env.FFMPEG_DIR ? path.join(process.env.FFMPEG_DIR, 'bin') : null;
}
/**
 * The environment of the engine: that of the application, with the folder of its libraries first
 * on the PATH, so that no other build of libav that lies earlier on it is loaded. In development,
 * the `ffmpeg` of that build makes the proxies, where nothing else is named: a release build of the
 * engine looks for one only beside itself.
 */
function engineEnvironment(program) {
  const env = process.platform === 'darwin' && app.isPackaged ? bundledEnvironment() : { ...process.env };
  const libraries = engineLibraries(program);
  if (libraries) {
    // Windows writes it Path, and reads it in any case.
    const key = Object.keys(env).find((name) => name.toUpperCase() === 'PATH') ?? 'PATH';
    env[key] = env[key] ? `${libraries}${path.delimiter}${env[key]}` : libraries;
    if (!app.isPackaged && !env.COLUMBINA_PROXY_FFMPEG) {
      env.COLUMBINA_PROXY_FFMPEG = path.join(libraries, process.platform === 'win32' ? 'ffmpeg.exe' : 'ffmpeg');
    }
  }
  return env;
}
/** What the service is told of the engine: the program of the window's, and in development the FFmpeg it is built with. */
function engineOfTheService() {
  const program = engineProgram();
  const told = program ? { COLUMBINA_ENGINE: program } : {};
  if (!app.isPackaged && process.env.FFMPEG_DIR) told.FFMPEG_DIR = process.env.FFMPEG_DIR;
  return told;
}
/**
 * The editing studio runs only on Windows for now (the author's decision; macOS will come later).
 * Elsewhere the shell starts no engine for the window, and every request for one is refused so.
 */
function refuseStudioElsewhere() {
  if (process.platform !== 'win32') throw new Error('The editing studio runs only on Windows for now. macOS will come later.');
}
/** Do something with the engine of the studio once all that was asked before is done. */
function inStudioTurn(operation) {
  const turn = studioOperation.then(operation);
  studioOperation = turn.then(() => undefined, () => undefined);
  return turn;
}
/** Tell the page of the engine of a session: only of the one that is open, or of its end. */
function tellStudio(number, what) {
  tell('columbina:engine-event', { ...what, session: number });
}
/**
 * Open a project folder in the studio: its project is `.columbina/studio` there, made where it
 * is not. The engine of a project that is open already is kept; one of another is closed first.
 * Answers the session: its number, its project, whether this application started its engine,
 * the process of the engine, and the revision that the engine said hello with.
 */
async function openStudio(cwd) {
  refuseStudioElsewhere();
  if (exiting) throw new Error('The application is closing.');
  const project = path.join(cwd, ...STUDIO_FOLDER);
  if (studio?.session.alive && directoryKey(studio.project) === directoryKey(project)) {
    // A page that was loaded again finds its engine as it was, and fetches the project anew; it
    // is told how the view stands, which it knew before it was loaded again.
    tellStudio(studio.number, { kind: 'opened', ...studio.opened });
    for (const viewer of Object.values(viewers)) if (viewer.told) tellStudio(studio.number, viewer.told);
    return studio.opened;
  }
  await closeStudio();
  if (exiting) throw new Error('The application is closing.');
  const named = process.env.COLUMBINA_ENGINE;
  if (!app.isPackaged && named && !path.isAbsolute(named)) {
    throw new Error(`COLUMBINA_ENGINE names the engine program by a path that is not whole, which is not taken: ${named}`);
  }
  const program = engineProgram();
  if (!program) {
    throw new Error(app.isPackaged
      ? 'This package has no engine of its own. Download it again from the releases of Columbina.'
      : 'There is no engine program. Build engine/ of the repository, or name the program with COLUMBINA_ENGINE by its whole path.');
  }
  mkdirSync(project, { recursive: true });
  keepOutOfHistory(project);
  studioNumber += 1;
  const number = studioNumber;
  let session;
  try {
    session = await engine.openEngine({
      program, project, env: engineEnvironment(program), logFile: engineLogPath(), kill: killTree,
      onEvent: (what) => {
        if (what.kind === 'exited' && studio?.number === number) {
          stopMigration(studio);
          studio = null;
          forgetView();
          tellScopesContext();
        }
        // The pictures of a scope of the window of the scopes go to it alone.
        if (what.kind === 'notification' && what.method === 'scopes.frame' && studio?.number === number
          && studio.windowScopes.has(what.params?.scope_id)) {
          tellScopes('columbina-scopes:event', { ...what, session: number });
          return;
        }
        if (what.kind === 'notification' && what.method === 'render.progress' && studio?.number === number) heardRender(studio, what.params);
        tellStudio(number, what);
      },
    });
  } catch (error) {
    tellStudio(number, { kind: 'error', message: error.message });
    throw error;
  }
  if (exiting) {
    session.abandon();
    throw new Error('The application is closing.');
  }
  const opened = { session: number, project, owned: session.owned, pid: session.pid, revision: session.hello.revision };
  // `jobs`: the exports that the window started, by their job, with the file that each writes; the
  // page may cancel those, and no other (an agent's), and have the file of one shown in its folder.
  // `migration`: the run of `video migrate` for the folder, while it runs.
  // `scopes`: the ids of the scopes that the page subscribed (see `keepScopes`).
  // `previewed`: whether the page asked for a Live Preview since one was last ended (see `endPreview`).
  studio = { number, project, session, opened, jobs: new Map(), migration: null, scopes: new Set(), windowScopes: new Set(), previewed: false };
  tellStudio(number, { kind: 'opened', ...opened });
  tellScopesContext();
  // A view is made anew for every engine, where the page has said where the viewer is.
  forgetView();
  for (const monitor of MONITORS) void attachView(monitor);
  migrateIn(cwd, studio);
  return opened;
}
/** Keep where an export of the window writes its film: the engine says so as it goes, and where it fell back to ProRes. */
function heardRender(open, params) {
  const job = open.jobs.get(params?.job);
  if (job && typeof params.path === 'string' && path.isAbsolute(params.path)) job.path = params.path;
}
/**
 * Turn the old projects of the folder into timelines of its studio, once its engine is open, as
 * the agent does the first time its tools work there: `video migrate` of the program of the service
 * reaches this engine through the lock of the project, and makes each a transaction of its own,
 * which the page sees as it is done. It runs beside the studio and holds up nothing, and whatever
 * fails is only said. The page is told what became of the old projects where there were any, or
 * why they could not be looked at. A folder where nothing was left to do is not looked at again
 * while the application runs: the command would do nothing there, and it takes a start of Python
 * to find that out.
 */
function migrateIn(cwd, open) {
  if (process.platform !== 'win32') return;
  const key = directoryKey(cwd);
  if (migrations.get(key) === 'done' || migrations.get(key) === 'running') return;
  let command = null;
  // Without a service there is nothing to run it with: a replay, or a package that lost its Python.
  try { command = programCommand(['video', 'migrate', '--workspace', cwd, '--json']); } catch { command = null; }
  if (!command) return;
  migrations.set(key, 'running');
  let child;
  try {
    // In the folder, as the service works in it: the program of the engine is never looked for there.
    child = spawn(command.executable, command.args, {
      cwd, env: { ...command.env, ...engineOfTheService() }, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
    });
  } catch (error) {
    migrations.set(key, 'again');
    tellStudio(open.number, { kind: 'migrated', migrations: [], error: `The old projects could not be looked at: ${error.message}` });
    return;
  }
  open.migration = child;
  const decoder = new StringDecoder('utf8');
  let written = '';
  let over = false;
  let said = '';
  let done = false;
  let timer = null;
  const finish = (document, failure) => {
    if (done) return;
    done = true;
    clearTimeout(timer);
    if (open.migration === child) open.migration = null;
    const reports = Array.isArray(document?.migrations) ? document.migrations.slice(0, MIGRATIONS_TOLD_MOST).map(migrationOf) : [];
    const error = failure ?? (typeof document?.error === 'string' ? short(document.error, MIGRATION_LINE_MOST) : null);
    // Where something could not be migrated, it is tried again the next time the studio is opened.
    migrations.set(key, error === null && reports.every((report) => report.error === undefined) ? 'done' : 'again');
    if (studio === open && (reports.length > 0 || error !== null)) tellStudio(open.number, { kind: 'migrated', migrations: reports, error });
  };
  timer = setTimeout(() => {
    killTree(child);
    finish(null, 'Looking at the old projects did not end in time.');
  }, MIGRATE_MS);
  child.stdout.on('data', (chunk) => {
    if (over) return;
    written += decoder.write(chunk);
    if (written.length > MIGRATE_OUTPUT_MOST) {
      over = true;
      written = '';
    }
  });
  // What it says besides its answer is told only where it gives none.
  child.stderr.on('data', (chunk) => { said = `${said}${chunk}`.slice(-MIGRATION_LINE_MOST); });
  child.on('error', (error) => finish(null, `The old projects could not be looked at: ${error.message}`));
  child.on('close', () => {
    // A migration that was stopped with its studio is told of no more.
    if (open.migration !== child && !done) {
      done = true;
      clearTimeout(timer);
      migrations.set(key, 'again');
      return;
    }
    if (over) {
      finish(null, 'The migration of the old projects said more than is read.');
      return;
    }
    let document = null;
    try { document = JSON.parse((written + decoder.end()).trim().split('\n').at(-1)); } catch { /* It wrote no document. */ }
    if (document === null || typeof document !== 'object' || Array.isArray(document)) {
      const last = said.trim().split('\n').at(-1)?.trim();
      finish(null, last ? short(last, MIGRATION_LINE_MOST) : 'The migration of the old projects ended without saying what it did.');
    } else finish(document, null);
  });
}
/** Stop the migration of a studio that is closed: what the engine took of it is found again by its notes the next time. */
function stopMigration(open) {
  const child = open?.migration;
  if (!child) return;
  open.migration = null;
  killTree(child);
}
/** What the page is told of one old project: its folder, the timeline it became and what was left out, or why it was not migrated. */
function migrationOf(report) {
  const text = (value) => (typeof value === 'string' ? short(value, MIGRATION_LINE_MOST) : null);
  const lines = (value) => (Array.isArray(value) ? value.map(text).filter((line) => line !== null).slice(0, MIGRATION_LINES_MOST) : []);
  const told = { source: text(report?.source) ?? '' };
  if (text(report?.error) !== null) {
    told.error = text(report.error);
    return told;
  }
  if (text(report?.timeline) !== null) told.timeline = text(report.timeline);
  told.dropped = lines(report?.dropped);
  told.warnings = lines(report?.warnings);
  return told;
}
/**
 * Write a file into the project of the studio that keeps what the engine makes there of its own,
 * its proxies and its lock, out of the history of the folder, where there is none. The project
 * itself is kept.
 */
function keepOutOfHistory(project) {
  try {
    writeFileSync(path.join(project, '.gitignore'), 'cache/\nengine.lock\n', { flag: 'wx' });
  } catch { /* There is one, which is the user's, or none can be written: the studio works all the same. */ }
}
/** Close the engine of the studio, if one is open: the engine of the application ends with it. */
async function closeStudio() {
  const open = studio;
  if (!open) return;
  stopMigration(open);
  studio = null;
  forgetView();
  tellScopesContext();
  await open.session.close();
}
/**
 * Subscribe a scope for the page, or let one go, keeping the ids of those it holds. The engine
 * sends a scope's pictures to the connection, which is the shell's and outlives a page loaded
 * again: the new page knows nothing of the old one's scopes, so `dropScopes` lets them go.
 */
async function keepScopes(open, method, params) {
  const result = await open.session.call(method, params);
  if (method === 'scopes.subscribe' && typeof result?.scope_id === 'string') open.scopes.add(result.scope_id);
  if (method === 'scopes.unsubscribe') open.scopes.delete(params?.scope_id);
  return result;
}
/** Tell the window of the scopes something, where it is open. */
function tellScopes(channel, value) {
  if (scopesWindow && !scopesWindow.isDestroyed() && !scopesWindow.webContents.isDestroyed()) scopesWindow.webContents.send(channel, value);
}
/** What the window of the scopes measures and speaks: the engine open, whether its viewer has something loaded, the language and theme. */
function scopesContext() {
  return { session: studio?.number ?? null, loaded: scopesAsked.loaded, language: scopesAsked.language, theme: preferences.theme };
}
function tellScopesContext() { tellScopes('columbina-scopes:context', scopesContext()); }
/** Let go the scopes of the window of the scopes, which is closed or loaded again. */
function dropWindowScopes() {
  windowScopesRound += 1;
  const open = studio;
  if (!open?.session.alive || !open.windowScopes?.size) { open?.windowScopes?.clear(); return; }
  for (const id of open.windowScopes) void open.session.call('scopes.unsubscribe', { scope_id: id }).catch(() => {});
  open.windowScopes.clear();
}
/** Where the window of the scopes was last, as its page kept it: whole numbers of a size it can be, on a display that is there. */
function scopesBounds(bounds) {
  const whole = (value, least, most) => (Number.isInteger(value) && value >= least && value <= most ? value : null);
  const width = whole(bounds?.width, 240, 8192);
  const height = whole(bounds?.height, 180, 8192);
  if (width === null || height === null) return {};
  const x = whole(bounds?.x, -32768, 32768);
  const y = whole(bounds?.y, -32768, 32768);
  if (x === null || y === null) return { width, height };
  let seen = true;
  try {
    const displays = screen.getAllDisplays?.() ?? [];
    seen = displays.length === 0 || displays.some(({ workArea: area }) => x < area.x + area.width && x + width > area.x && y < area.y + area.height && y + 32 > area.y);
  } catch { /* Where the displays cannot be read, the window goes where it was. */ }
  return seen ? { x, y, width, height } : { width, height };
}
/**
 * The floating window of the video scopes of the Color page (Reference Manual 20, pp. 3007–3008,
 * 3018–3019: Workspace › Video Scopes, Ctrl+Shift+W): a window of its own, over the main one, which
 * the engine's view of the viewer does not cover and which may go to another display. Its page
 * reaches the engine for its own scopes alone; their pictures go to it alone. It is closed with the
 * main window, and as the main page is loaded again.
 */
function openScopesWindow(asked) {
  scopesAsked = { loaded: asked?.loaded === true, language: spoken(asked?.language) };
  if (scopesWindow) { tellScopesContext(); return; }
  const opened = new BrowserWindow({
    parent: window, width: 640, height: 420, minWidth: 240, minHeight: 180, ...scopesBounds(asked?.bounds),
    show: false, backgroundColor: titleBarThemes[preferences.theme].color, title: 'Columbina', icon: path.join(__dirname, 'icon.ico'),
    webPreferences: { preload: path.join(__dirname, 'scopes-preload.cjs'), contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true },
  });
  scopesWindow = opened;
  opened.setMenuBarVisibility(false);
  closeToTheWorld(opened);
  opened.once('ready-to-show', () => opened.show());
  opened.webContents.on('render-process-gone', dropWindowScopes);
  opened.webContents.on('did-start-navigation', (details, _url, inPlace, mainFrame) => {
    const main = details?.isMainFrame ?? mainFrame;
    const same = details?.isSameDocument ?? inPlace;
    if (main === true && same !== true) dropWindowScopes();
  });
  opened.once('closed', () => {
    if (scopesWindow !== opened) return;
    scopesWindow = null;
    dropWindowScopes();
    // The page shows its scopes docked again.
    tellStudio(studio?.number ?? null, { kind: 'scopes-window', open: false });
  });
  void opened.loadFile(scopesPath);
}
function closeScopesWindow() {
  const opened = scopesWindow;
  if (opened && !opened.isDestroyed()) opened.close();
}
/** Let go the scopes of a page that is loaded again or gone, whose pictures would go to no one. */
function dropScopes() {
  const open = studio;
  if (!open?.session.alive || !open.scopes?.size) return;
  for (const id of open.scopes) void open.session.call('scopes.unsubscribe', { scope_id: id }).catch(() => {});
  open.scopes.clear();
}
/**
 * End the Live Preview of a page that is loaded again or gone (`viewer.preview`, p. 3411): the
 * engine keeps it for the connection, which is the shell's, and the new page knows nothing of it.
 */
function endPreview() {
  const open = studio;
  if (!open?.session.alive || !open.previewed) return;
  open.previewed = false;
  void open.session.call('viewer.preview', { on: false }).catch(() => {});
}
/** Forget the view of an engine that is gone: one is made anew for the next. */
function forgetView() {
  for (const viewer of Object.values(viewers)) {
    viewer.state = 'none';
    viewer.told = null;
    viewer.sent = true;
    viewer.generation += 1;
  }
}
/** Tell the page how the view stands, and remember it for a page that is loaded again. */
function tellView(open, monitor, what) {
  const event = { ...what, monitor };
  viewers[monitor].told = event;
  tellStudio(open.number, event);
}
/** The handle of the window as the engine reads it: a number written in decimal. */
function windowHandle() {
  const handle = window.getNativeWindowHandle();
  return (handle.length >= 8 ? handle.readBigUInt64LE(0) : BigInt(handle.readUInt32LE(0))).toString();
}
/**
 * What the page is told of a view that could not be made, or moved: none on this system, or why
 * not. `generation` is the number of the view that the request was about.
 */
function viewRefused(open, monitor, error, generation) {
  const viewer = viewers[monitor];
  if (studio !== open || generation !== viewer.generation) return;
  const reason = error?.data?.reason;
  if ((error?.code === -32003 && reason === 'unsupported') || error?.code === -32601) {
    viewer.state = 'unsupported';
    // An engine without a window, such as an agent's that had the project open, has no view.
    tellView(open, monitor, { kind: 'unsupported', reason: error.code === -32601 ? 'headless' : 'unsupported', message: error.message });
  } else if (error?.code === -32003 && (reason === 'gone' || reason === 'no_view')) {
    // The view was let go with the window it was in, or never made: it is made again. What is
    // said late of a view that was made before the last one is of no view that there is.
    if (generation !== viewer.generation || viewer.state !== 'attached') return;
    viewer.state = 'none';
    void attachView(monitor);
  } else {
    viewer.state = 'failed';
    tellView(open, monitor, { kind: 'error', message: error?.message ?? String(error), code: error?.code ?? null, data: error?.data });
  }
}
/**
 * Make the view of the engine in the window, over the viewer, where the page has said where it is.
 * Only the shell sends this: it holds the handle of the window. A view is made shown, so none is
 * made while the page wants it hidden (something lies over the viewer, or the studio is left):
 * it is made once the page wants it shown.
 */
async function attachView(monitor) {
  const viewer = viewers[monitor];
  const open = studio;
  if (!open || viewer.rect === null || viewer.state !== 'none' || !viewer.shown || viewer.blank || !window || window.isDestroyed()) return;
  viewer.state = 'attaching';
  viewer.generation += 1;
  const generation = viewer.generation;
  const rect = viewer.rect;
  let attached;
  try {
    attached = await open.session.view('view.attach', { monitor, parent: windowHandle(), ...rect });
  } catch (error) {
    viewRefused(open, monitor, error, generation);
    return;
  }
  if (studio !== open) return;
  viewer.state = 'attached';
  // A view is made shown. What the page said while it was being made is done at once, before
  // anything else: hidden, if something came to lie over the viewer meanwhile.
  viewer.sent = true;
  showView(monitor);
  if (viewer.rect !== rect) placeView(monitor);
  tellView(open, monitor, { kind: 'attached', adapter: attached?.adapter ?? null, backend: attached?.backend ?? null });
}
/** Move the view where the page keeps the viewer now. */
function placeView(monitor) {
  const viewer = viewers[monitor];
  const open = studio;
  if (!open || viewer.state !== 'attached') return;
  const { generation } = viewer;
  open.session.view('view.place', { monitor, ...viewer.rect }).catch((error) => viewRefused(open, monitor, error, generation));
}
/**
 * Show the view, or hide it: while something of the page lies over the viewer, and while the page
 * is loaded again, until it says where the viewer is.
 */
function showView(monitor) {
  const viewer = viewers[monitor];
  const open = studio;
  const shown = viewer.shown && !viewer.blank;
  if (!open || viewer.state !== 'attached' || viewer.sent === shown) return;
  viewer.sent = shown;
  const { generation } = viewer;
  open.session.view('view.show', { monitor, shown }).catch((error) => viewRefused(open, monitor, error, generation));
}
/**
 * Hide the view while the page is gone or being loaded again: the native window would stay over
 * whatever the page shows meanwhile. The new page wants it shown once it says where the viewer is.
 */
function blankView() {
  for (const monitor of MONITORS) {
    viewers[monitor].blank = true;
    viewers[monitor].shown = true;
    showView(monitor);
  }
}
function monitorOf(monitor) {
  if (!MONITORS.includes(monitor)) throw new Error('A monitor is source or timeline.');
  return monitor;
}
/** A rectangle of the viewer as the page gives it: whole pixels of the screen, from the corner of the client area. */
function viewRectOf(given) {
  const whole = (value, most) => Number.isSafeInteger(value) && value >= 0 && value <= most;
  if (given === null || typeof given !== 'object' || !whole(given.x, VIEW_PLACE_MOST) || !whole(given.y, VIEW_PLACE_MOST)
    || !whole(given.width, VIEW_SIZE_MOST) || !whole(given.height, VIEW_SIZE_MOST)) {
    throw new Error('The viewer lies at whole pixels of the screen, from the corner of the window, and is at most 16384 pixels wide and high.');
  }
  return { x: given.x, y: given.y, width: given.width, height: given.height };
}
/** Take where the page keeps the viewer now: the view is made there, or moved there. */
function takeViewRect(given) {
  const rect = viewRectOf(given);
  const monitor = monitorOf(given?.monitor);
  const viewer = viewers[monitor];
  viewer.rect = rect;
  viewer.blank = false;
  if (viewer.state === 'none') void attachView(monitor);
  else {
    placeView(monitor);
    showView(monitor);
  }
}
/** Take whether the view is to be shown. */
function takeViewShown(given) {
  const shown = given?.shown;
  if (typeof shown !== 'boolean') throw new Error('The view is shown or not.');
  const monitor = monitorOf(given?.monitor);
  const viewer = viewers[monitor];
  viewer.shown = shown;
  if (viewer.state === 'none') void attachView(monitor);
  else showView(monitor);
}
/** The most subtitle tracks that an export may save as SRT. */
const MOST_SUBTITLE_TRACKS = 64;
/**
 * Export settings contain no paths. The engine validates timeline, range and subtitle tracks
 * against its snapshot. `subtitle_tracks` names the subtitle tracks saved beside the film as SRT
 * (none where it is left out): ids, each once.
 */
function renderOptionsOf(given) {
  if (!given || typeof given.timeline !== 'string' || given.timeline.length === 0
    || !['h264', 'hevc', 'prores'].includes(given.codec) || !['high', 'medium', 'small'].includes(given.quality)) {
    throw new Error('Name a timeline, codec and quality for export.');
  }
  const tracks = given.subtitle_tracks ?? [];
  if (!Array.isArray(tracks) || tracks.length > MOST_SUBTITLE_TRACKS
    || !tracks.every((id) => typeof id === 'string' && id.length > 0 && id.length <= 256)) {
    throw new Error('Subtitle tracks for export are a list of track ids.');
  }
  const options = { timeline: given.timeline, codec: given.codec, quality: given.quality, subtitle_tracks: [...new Set(tracks)] };
  // The name of the timeline is only what the dialog suggests to name the file after.
  if (given.range !== undefined) {
    const { from, to } = given.range ?? {};
    if (!Number.isSafeInteger(from) || from < 0 || !Number.isSafeInteger(to) || to <= from) throw new Error('An export range is a nonempty interval of whole frames.');
    options.range = { from, to };
  }
  return options;
}
/** A name of a file made from the name of a timeline: without what Windows refuses in a name. */
function fileNameOf(name) {
  const cleaned = (typeof name === 'string' ? name : '').replace(/[<>:"/\\|?*\u0000-\u001f]/g, '_').replace(/[. ]+$/, '').trim().slice(0, 120);
  // A name that Windows keeps for a device is no name of a file.
  return !cleaned ? 'export' : /^(con|prn|aux|nul|com\d|lpt\d)$/i.test(cleaned) ? `_${cleaned}` : cleaned;
}
/**
 * The names of the SRT files that an export of `timeline` to a film named `stem` writes for the
 * subtitle tracks `chosen`, as the engine names them (`srt.rs`): in the order of the tracks, one is
 * `<stem>.srt`, more are `<stem>.<language, else name>.srt`, made safe and told apart by `-2`, `-3`.
 */
function srtNamesOf(timeline, chosen, stem) {
  const tracks = (Array.isArray(timeline?.tracks) ? timeline.tracks : [])
    .filter((track) => track?.kind === 'subtitle' && chosen.includes(track.id));
  if (tracks.length === 1) return [`${stem}.srt`];
  const safe = (label) => {
    const name = Array.from(String(label)).slice(0, 80)
      .map((ch) => (/[\u0000-\u001f\u007f-\u009f]/.test(ch) || '<>:"/\\|?*'.includes(ch) ? '_' : ch)).join('')
      .replace(/^[. ]+|[. ]+$/g, '');
    return name || 'subtitles';
  };
  const taken = new Set();
  return tracks.map((track) => {
    const label = safe(typeof track.language === 'string' && track.language ? track.language : track.name ?? '');
    let name = label;
    for (let suffix = 2; taken.has(name.toLowerCase()); suffix += 1) name = `${label}-${suffix}`;
    taken.add(name.toLowerCase());
    return `${stem}.${name}.srt`;
  });
}
/**
 * The SRT files an export to `chosen` writes, by the timeline in the project of the engine (`names`;
 * null where the engine does not tell it, and then every `<stem>.*.srt` is taken to be one), and the
 * revision of the project that they were worked out at (`revision`, null where none was read), which
 * render.start is held to.
 */
async function srtFilesOf(open, chosen, options) {
  if (options.subtitle_tracks.length === 0) return { names: [], revision: null };
  try {
    const got = await open.session.call('project.get');
    const timeline = got?.project?.timelines?.find?.((each) => each?.id === options.timeline);
    const revision = Number.isSafeInteger(got?.revision) && got.revision >= 0 ? got.revision : null;
    return timeline
      ? { names: srtNamesOf(timeline, options.subtitle_tracks, path.parse(chosen).name), revision }
      : { names: null, revision: null };
  } catch {
    return { names: null, revision: null };
  }
}
/** How many times an export whose SRT names went stale before the engine took it is prepared again. */
const RENDER_TRIES = 3;
/**
 * The files in the folder of `chosen`, read without holding up the main process, where `chosen` is
 * there (Save As asked whether to replace it); null where it is not, or the folder cannot be read,
 * and then nothing is replaced.
 */
async function besideOf(chosen) {
  if (!existsSync(chosen)) return null;
  try { return (await readdir(path.dirname(chosen))).map(String); } catch { return null; }
}
/**
 * Whether an export to `chosen` may replace what is there, given the files of its folder (`there`,
 * see `besideOf`). Save As asked the user whether to replace the file that they chose (Windows asks
 * it of a file that is there), so a file that is there was confirmed. What the export would write
 * beside it was not asked of: an SRT of a subtitle track chosen (`srts`, the names that it writes;
 * null where they are not known, and any SRT of the film's name counts), the .mov of a ProRes
 * fallback, or a partial file. Where any of those is there, nothing is replaced, and `blocking` is
 * that file, which the export is refused for, as the engine refuses it without the dialog.
 */
function replacing(chosen, options, there, srts = null) {
  if (there === null) return { replace: false, blocking: null };
  const { dir, name, ext } = path.parse(chosen);
  const same = process.platform === 'win32' ? (text) => text.toLowerCase() : (text) => text;
  const stem = same(name);
  const beside = (file) => file === same(`${name}${ext}.part`)
    || (srts === null
      ? options.subtitle_tracks.length > 0 && file.startsWith(`${stem}.`) && (file.endsWith('.srt') || file.endsWith('.srt.part'))
      : srts.some((srt) => file === same(srt) || file === same(`${srt}.part`)))
    || (options.codec !== 'prores' && same(ext) === '.mp4' && (file === `${stem}.mov` || file === `${stem}.mov.part`));
  const found = there.find((file) => beside(same(file)));
  return found === undefined ? { replace: true, blocking: null } : { replace: false, blocking: path.join(dir, found) };
}
/**
 * Look through a transaction of the page: it may name no file. Media is added only from files that
 * the user chose in the dialog (`media.import`), and a proxy is set only by the engine, which makes
 * it: the engine opens the file that a path names, and deletes a proxy that it cannot read.
 */
function transactionOfThePage(params) {
  const ops = params?.ops;
  if (!Array.isArray(ops)) throw new Error('A transaction of the page is a list of operations.');
  for (const op of ops) {
    if (op === null || typeof op !== 'object' || Array.isArray(op)) throw new Error('An operation is an object.');
    if (op.op === 'add_media') throw new Error('The page adds no media itself: media is imported from files that are chosen in the dialog.');
    // Whether media is a still picture is a fact of its file, which the engine found as it imported
    // it (or a project made before told); the page does not change it.
    if (op.op === 'set_media' && op.still !== undefined) throw new Error('The page does not say whether media is a still picture: the engine knows it from the file.');
    if (op.op === 'set_proxy' && op.proxy !== null && op.proxy !== undefined) {
      throw new Error('The page sets no proxy: the engine makes them. It may only take one away.');
    }
  }
}
/**
 * A comparison of the viewer that the page asks for: its references name stills by their ids and
 * the timeline by its clips and frames, never a file, which the engine would read.
 */
function comparisonOfThePage(params) {
  const references = [params?.reference, ...(Array.isArray(params?.items) ? params.items : [])];
  for (const reference of references) {
    if (reference !== null && typeof reference === 'object' && Object.hasOwn(reference, 'path')) {
      throw new Error('The page names a still of the Gallery by its id, not a file.');
    }
  }
}
/**
 * What the page is answered of a request to the engine: the result, or the error as the engine
 * gave it, with its code and data, which an error thrown to the page would lose.
 */
function answered(asking) {
  return asking.then(
    (result) => ({ ok: true, result }),
    (error) => ({ ok: false, error: { code: error?.code ?? null, message: error?.message ?? String(error), data: error?.data ?? null } }),
  );
}
/**
 * Another command of the program, such as `setup --list`, made from the command of the service
 * process. Null where there is none to make it from: in a replay, or where the command does not serve.
 */
function programCommand(words) {
  const config = serviceCommand();
  const serves = config.args.indexOf('serve');
  if (config.mode !== 'service' || serves < 0) return null;
  // What follows `serve` was meant for serving, such as a profile.
  return { executable: config.executable, args: [...config.args.slice(0, serves), ...words], env: config.env };
}
// What is written through the setup program, one write after the other.
let writing = Promise.resolve();
/**
 * Have the setup program write something once all that was asked to be written before is
 * written. A write reads all that is stored and writes it back, so two at once would lose what
 * one of them wrote. A write that fails holds up none after it.
 */
function inTurn(write) {
  const turn = writing.then(write);
  writing = turn.then(() => undefined, () => undefined);
  return turn;
}
/**
 * Run a command of the program in a form that asks nothing, such as `setup --list`, and return
 * the document that it answers with.
 *
 * What it is given goes to its standard input and nowhere else: not to its command line, not
 * into its environment, not into a file. That is how a key reaches it. Of what it writes, so
 * many characters are read at most.
 */
async function runProgram(words, input = '', most = SETUP_OUTPUT_MOST) {
  if (askingLoginShell) await askingLoginShell;
  const command = programCommand(words);
  if (!command) throw new Error(NO_SETUP);
  return new Promise((resolve, reject) => {
    const child = spawn(command.executable, command.args, {
      cwd: app.getPath('home'), env: command.env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
    });
    const decoder = new StringDecoder('utf8');
    let written = '';
    let over = false;
    let done = false;
    const settle = (act, value) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      act(value);
    };
    const timer = setTimeout(() => {
      killTree(child);
      settle(reject, new Error('The setup program did not end in time.'));
    }, SETUP_MS);
    child.stdout.on('data', (chunk) => {
      if (over) return;
      written += decoder.write(chunk);
      if (written.length > most) {
        over = true;
        written = '';
      }
    });
    // What a person would be shown. It is taken so that the program is not held up, and kept nowhere.
    child.stderr.on('data', () => {});
    // A program that ended before it read is told of by how it ended.
    child.stdin.on('error', () => {});
    child.on('error', (error) => settle(reject, new Error(`The setup program could not be started: ${error.message}`)));
    child.on('close', () => {
      if (over) {
        settle(reject, new Error('The setup program wrote more than is read.'));
        return;
      }
      let document = null;
      try { document = JSON.parse((written + decoder.end()).trim().split('\n').at(-1)); } catch { /* It wrote no document. */ }
      if (document === null || typeof document !== 'object' || Array.isArray(document)) {
        settle(reject, new Error('The setup program did not answer with a document.'));
      } else {
        settle(resolve, document);
      }
    });
    // Its input is closed after what it is given, so that it waits for nothing more.
    child.stdin.end(input);
  });
}
/** The folder of the project that the service of the moment works in, or null where none runs or it replays. */
function projectOfTheService() {
  // The service of the project is gone once there is another service, or none.
  return serviceProject?.child === service && serviceProject.mode === 'service' ? serviceProject.cwd : null;
}
/**
 * Hold files that the user chose or dropped, each where it is a file and not too large. Returns
 * what the page is told: the files that are held, by id, name and size, and the names of the
 * others with why they are not. How many go with one message, the page sees to.
 */
async function attach(paths) {
  const attached = [];
  const refused = [];
  for (const chosen of paths) {
    const name = path.basename(chosen);
    let found;
    try {
      found = await stat(chosen);
    } catch {
      refused.push({ name, why: 'gone' });
      continue;
    }
    if (!found.isFile()) refused.push({ name, why: 'notAFile' });
    else if (found.size > ATTACHMENT_MOST_BYTES) refused.push({ name, why: 'tooBig' });
    else if (attachments.size >= ATTACHMENTS_HELD_MOST) refused.push({ name, why: 'tooMany' });
    else {
      const id = `attachment-${++attachmentNumber}`;
      attachments.set(id, { path: chosen, name, size: found.size });
      attached.push({ id, name, size: found.size });
    }
  }
  return { attached, refused };
}
/** The name of the folder that the files of one message are kept in: when they were sent, to the second. */
function sentAt(date) {
  const two = (number) => String(number).padStart(2, '0');
  return `${date.getFullYear()}${two(date.getMonth() + 1)}${two(date.getDate())}-`
    + `${two(date.getHours())}${two(date.getMinutes())}${two(date.getSeconds())}`;
}
/**
 * Copy a file into a folder under its name, or under the name with a number where one of that name
 * is there already. What cannot be seen, such as the end of a line, is no part of the name: the
 * message names each file on a line of its own. Returns the name that it has there.
 */
async function copiedInto(folder, file) {
  const name = file.name.replace(/[\u0000-\u001f\u007f]/g, '_');
  const dot = name.lastIndexOf('.');
  const [stem, ending] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ''];
  for (let number = 1; number <= ATTACHMENTS_HELD_MOST; number += 1) {
    const taken = number === 1 ? name : `${stem} (${number})${ending}`;
    try {
      await copyFile(file.path, path.join(folder, taken), constants.COPYFILE_EXCL);
      return taken;
    } catch (error) {
      if (error?.code !== 'EEXIST') throw error;
    }
  }
  throw new Error(`There is no name left for ${file.name}.`);
}
/**
 * Where the shell asks which version is the newest: GitHub, in a package. From source it asks
 * only where the environment names another address, which the walks through the window use,
 * and nowhere otherwise.
 */
function releasesFeed() {
  if (app.isPackaged) return RELEASES;
  const named = process.env.COLUMBINA_UPDATE_FEED;
  return typeof named === 'string' && /^https?:\/\/\S+$/.test(named) ? named : null;
}
/** The numbers of a version as a release names it, with what goes before them, or null for no such version. */
function versionOf(text, before = '') {
  const found = typeof text === 'string' ? new RegExp(`^${before}(\\d{1,6})\\.(\\d{1,6})\\.(\\d{1,6})$`).exec(text) : null;
  return found ? found.slice(1).map(Number) : null;
}
/** Whether one version comes after another. */
function later(one, other) {
  for (let at = 0; at < 3; at += 1) if (one[at] !== other[at]) return one[at] > other[at];
  return false;
}
/**
 * Ask GitHub which release of the desktop application is the newest. Returns what the page is
 * told: the version that comes after the one that runs, with the page of its release, or none,
 * or why it is not known. A release of the program alone, a draft, one that is tried before it
 * is released and a tag of any other shape are not releases of the application.
 */
async function newerRelease(feed) {
  const none = { newer: null, page: null, failure: null };
  const failed = (failure) => ({ ...none, failure });
  let response;
  let text;
  try {
    releasesSession ??= session.fromPartition(RELEASES_PARTITION);
    response = await releasesSession.fetch(feed, {
      headers: { Accept: 'application/vnd.github+json', 'User-Agent': `Columbina/${app.getVersion()}` },
      signal: AbortSignal.timeout(UPDATE_MS),
    });
    // GitHub turns away who asks too often, for an hour at most.
    if (response.status === 403 || response.status === 429) return failed('busy');
    if (!response.ok) return failed('unreadable');
    text = await response.text();
  } catch {
    return failed('network');
  }
  let releases = null;
  try { releases = text.length > UPDATE_OUTPUT_MOST ? null : JSON.parse(text); } catch { /* No list, as below. */ }
  if (!Array.isArray(releases)) return failed('unreadable');
  let newest = null;
  for (const release of releases) {
    if (release === null || typeof release !== 'object' || release.draft !== false || release.prerelease !== false) continue;
    const version = versionOf(release.tag_name, 'desktop-v');
    if (version !== null && (newest === null || later(version, newest))) newest = version;
  }
  const running = versionOf(app.getVersion());
  if (newest === null || running === null || !later(newest, running)) return none;
  const newer = newest.join('.');
  return { newer, page: `${RELEASE_PAGE}${newer}`, failure: null };
}
/** The names of the services in what `setup --list` answered, and the pages on which their keys are made. */
function namesAndPages(list) {
  const services = Array.isArray(list?.services) ? list.services : [];
  const text = (value) => (typeof value === 'string' && value ? [value] : []);
  return {
    names: services.flatMap((service) => text(service?.name)),
    pages: services.flatMap((service) => text(service?.key_page)),
  };
}
/** Where the user keeps pets of their own, and the file in which the service process tells what its task does. */
const userPets = () => path.join(app.getPath('userData'), 'pets');
const petStateFile = () => path.join(app.getPath('userData'), 'pet-state.json');
/** The pets that can be chosen: those that come with the application, and those of the user. */
function petList() {
  return pets.listPets([{ folder: petsBuiltIn, origin: 'builtin' }, { folder: userPets(), origin: 'user' }]);
}
/** The pet that was chosen, or the first one where that one is gone, or none. */
function chosenPet() {
  const all = petList();
  return all.find((one) => one.key === preferences.pet) ?? all[0] ?? null;
}
/** What the window of the pet is given of a pet: its picture and the rows that its package names. */
function petSheet() {
  const pet = chosenPet();
  if (!pet) return null;
  try {
    return { sheet: pets.sheetOf(pet), animations: pet.animations, name: pet.name };
  } catch {
    return null;
  }
}
function tellPet(channel, value) {
  if (petWindow && !petWindow.isDestroyed() && !petWindow.webContents.isDestroyed()) petWindow.webContents.send(channel, value);
}
/** Show the pet, where it stood last, over the other windows. */
function openPet() {
  if (petWindow) return;
  const opened = new BrowserWindow({
    ...PET_SIZE, ...(preferences.petAt ?? {}), transparent: true, frame: false, resizable: false, minimizable: false,
    maximizable: false, fullscreenable: false, skipTaskbar: true, alwaysOnTop: true, hasShadow: false, show: false,
    backgroundColor: '#00000000', title: 'Columbina',
    webPreferences: { preload: path.join(__dirname, 'pet-preload.cjs'), contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true },
  });
  petWindow = opened;
  opened.setMenuBarVisibility(false);
  closeToTheWorld(opened);
  opened.once('ready-to-show', () => opened.showInactive());
  opened.once('closed', () => { if (petWindow === opened) stopLooking(); });
  void opened.loadFile(petPath);
  lookAtTask();
  petLooking = setInterval(lookAtTask, PET_LOOK_MS);
}
function stopLooking() {
  petWindow = null;
  petDrag = null;
  clearInterval(petLooking);
  petLooking = null;
}
function closePet() {
  const opened = petWindow;
  stopLooking();
  if (opened && !opened.isDestroyed()) opened.destroy();
}
/** Whether a process is there: one that is somebody else's is there too. */
function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === 'EPERM';
  }
}
/**
 * Look at what the service process wrote of its task, and tell the pet where that changed, with
 * what it says in its bubble in the language of the page. That the task is done is said where the
 * window is not looked at.
 */
function lookAtTask() {
  let text = null;
  try { text = readFileSync(petStateFile(), 'utf8'); } catch { /* No task has been told of yet. */ }
  const told = pets.stateOf(text, alive);
  if (!told) return;
  const words = trayWords();
  const bubble = told.state === 'waiting' ? words.petWaiting(told.label)
    : told.state === 'failed' ? words.petFailed
      : told.state === 'done' && !window?.isFocused() ? words.petDone : '';
  if (told.state === petShown.state && told.label === petShown.label && bubble === petShown.bubble) return;
  petShown = { ...told, bubble };
  tellPet('columbina-pet:shown', petShown);
}
function installHandlers() {
  ipcMain.handle('columbina:select-folder', async (event, asked) => {
    checkSender(event);
    // The page names its language and nothing more: the words of the dialog are the shell's own.
    const title = WORDS[spoken(asked?.language)].chooseFolder;
    const result = await dialog.showOpenDialog(window, { title, properties: ['openDirectory'] });
    if (result.canceled || !result.filePaths[0]) return null;
    const cwd = realpathSync(result.filePaths[0]);
    if (!statSync(cwd).isDirectory()) throw new Error('The selected path is not a folder.');
    approvedDirectories.add(directoryKey(cwd));
    remember(cwd);
    return cwd;
  });
  ipcMain.handle('columbina:recent-projects', (event) => {
    checkSender(event);
    return preferences.recent.filter(isFolder).map((folder) => ({ path: folder, name: path.basename(folder) }));
  });
  ipcMain.handle('columbina:open-recent', (event, asked) => {
    checkSender(event);
    // Only a folder that the user chose in the dialog at an earlier time, and that the shell wrote down then.
    const kept = typeof asked === 'string'
      ? preferences.recent.find((known) => directoryKey(known) === directoryKey(asked)) : undefined;
    if (kept === undefined) throw new Error('That folder is not among the recent projects.');
    const cwd = realpathSync(kept);
    if (directoryKey(cwd) !== directoryKey(kept)) throw new Error('That folder leads somewhere else than when it was chosen.');
    if (!statSync(cwd).isDirectory()) throw new Error('The selected path is not a folder.');
    approvedDirectories.add(directoryKey(cwd));
    remember(cwd);
    return cwd;
  });
  ipcMain.handle('columbina:start', async (event, options) => {
    checkSender(event);
    // Validate before queueing so an unapproved request cannot stop a running project.
    approvedDirectory(options);
    return enqueueService(() => startServiceIn(() => approvedDirectory(options), []));
  });
  ipcMain.handle('columbina:start-chat', async (event) => {
    checkSender(event);
    // A conversation that belongs to no project, in the folder that the shell names for it. Its
    // agent changes nothing there: the profile chat gives it no tool for that.
    return enqueueService(() => startServiceIn(() => {
      const folder = chatFolder();
      mkdirSync(folder, { recursive: true });
      return folder;
    }, ['--profile', 'chat']));
  });
  ipcMain.on('columbina:send', (event, line) => {
    try {
      checkSender(event);
      if (typeof line !== 'string' || /[\r\n]/.test(line)) throw new Error('Each protocol message must occupy one line.');
      if (!service?.stdin.writable) throw new Error('Service is not connected.');
      service.stdin.write(`${line}\n`, 'utf8');
    } catch (error) { tell('columbina:closed', error.message); }
  });
  ipcMain.handle('columbina:stop', async (event) => {
    checkSender(event);
    await enqueueService(stopService);
    if (exiting) finishWindowClose();
  });
  ipcMain.handle('columbina:local-music', async (event) => {
    checkSender(event);
    const musicPath = path.join(realpathSync(app.getPath('userData')), 'music', 'somnias-a-luna.m4a');
    let file;
    try {
      if (directoryKey(await realpath(musicPath)) !== directoryKey(musicPath)) {
        throw new Error('The default music file must stay in the application music folder.');
      }
      file = await open(musicPath, 'r');
      const details = await file.stat();
      if (!details.isFile() || details.size > 64 * 1024 * 1024) throw new Error('The default music file is too large or is not a file.');
      const bytes = await file.readFile();
      return { name: 'Somnias a Luna', mimeType: 'audio/mp4', bytes: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) };
    } catch (error) {
      if (error.code === 'ENOENT') return null;
      throw error;
    } finally { await file?.close(); }
  });
  ipcMain.handle('columbina:set-theme', (event, theme) => {
    checkSender(event);
    if (!Object.hasOwn(titleBarThemes, theme)) throw new Error('Unknown application theme.');
    if (process.platform === 'win32' || process.platform === 'linux') window.setTitleBarOverlay(titleBarThemes[theme]);
    window.setBackgroundColor(titleBarThemes[theme].color);
    // The next start opens in this theme, before the page has read anything.
    if (preferences.theme !== theme) {
      preferences.theme = theme;
      keepPreferences();
    }
  });
  ipcMain.handle('columbina:pets', (event) => {
    checkSender(event);
    const chosen = chosenPet();
    return {
      on: preferences.petOn, chosen: chosen?.key ?? null,
      pets: petList().map(({ key, name, description, origin }) => ({ key, name, description, origin })),
      folder: userPets(), stateFile: petStateFile(),
    };
  });
  ipcMain.handle('columbina:set-pet', (event, asked) => {
    checkSender(event);
    if (typeof asked?.on === 'boolean') preferences.petOn = asked.on;
    if (typeof asked?.chosen === 'string' && petList().some((one) => one.key === asked.chosen)) {
      preferences.pet = asked.chosen;
      tellPet('columbina-pet:sheet', petSheet());
    }
    keepPreferences();
    if (preferences.petOn) openPet();
    else closePet();
  });
  ipcMain.handle('columbina:open-pets-folder', async (event) => {
    checkSender(event);
    mkdirSync(userPets(), { recursive: true });
    const failed = await shell.openPath(userPets());
    if (failed) throw new Error(failed);
  });
  ipcMain.handle('columbina-pet:ready', (event) => {
    checkPetSender(event);
    return { pet: petSheet(), shown: petShown };
  });
  ipcMain.handle('columbina-pet:click', (event) => {
    checkPetSender(event);
    showWindow();
  });
  ipcMain.handle('columbina-pet:drag', (event, phase) => {
    checkPetSender(event);
    if (phase === 'start') {
      petDrag = { cursor: screen.getCursorScreenPoint(), at: petWindow.getPosition() };
      return;
    }
    if (!petDrag || (phase !== 'move' && phase !== 'end')) return;
    const cursor = screen.getCursorScreenPoint();
    const x = petDrag.at[0] + cursor.x - petDrag.cursor.x;
    const y = petDrag.at[1] + cursor.y - petDrag.cursor.y;
    petWindow.setPosition(x, y);
    if (phase === 'end') {
      petDrag = null;
      preferences.petAt = { x, y };
      keepPreferences();
    }
  });
  ipcMain.handle('columbina-pet:menu', (event) => {
    checkPetSender(event);
    const words = trayWords();
    Menu.buildFromTemplate([
      { label: words.open, click: showWindow },
      { label: words.hidePet, click: () => { preferences.petOn = false; keepPreferences(); closePet(); tell('columbina:pet', { on: false }); } },
    ]).popup({ window: petWindow });
  });
  ipcMain.handle('columbina:set-language', (event, language) => {
    checkSender(event);
    if (typeof language !== 'string' || !Object.hasOwn(WORDS, language)) throw new Error('Unknown language of the page.');
    // The menu of the tray speaks it, also at the next start before the page has said it.
    if (preferences.language !== language) {
      preferences.language = language;
      keepPreferences();
    }
    if (tray) trayMenu();
  });
  ipcMain.handle('columbina:login-item', (event) => {
    checkSender(event);
    return loginItem();
  });
  ipcMain.handle('columbina:set-login-item', (event, on) => {
    checkSender(event);
    if (typeof on !== 'boolean') throw new Error('Starting with the system is on or off.');
    if (!loginItemOffered()) throw new Error('This application does not start with the system.');
    app.setLoginItemSettings({ openAtLogin: on, path: process.execPath, args: [HIDDEN] });
    return loginItem();
  });
  ipcMain.handle('columbina:service-tail', (event) => {
    checkSender(event);
    // A line that is still being written is told of as far as it has come.
    const rest = plain(lastWords.rest);
    return rest ? [...lastWords.lines.slice(1 - TAIL_LINES), short(rest, TAIL_LINE_CHARS)] : [...lastWords.lines];
  });
  ipcMain.handle('columbina:show-log', (event) => {
    checkSender(event);
    shell.showItemInFolder(logPath());
  });
  ipcMain.handle('columbina:notify', (event, message) => {
    checkSender(event);
    // What is shown may hold what a model wrote, such as a command: one line, and not a long one.
    const said = (value, most) => (typeof value === 'string' ? short(plain(value), most) : '');
    const title = said(message?.title, 80);
    if (!title) throw new Error('A notification needs a title.');
    // Whoever looks at the window sees it there.
    if (window.isFocused()) return false;
    window.flashFrame(true);
    if (!flashing) {
      flashing = true;
      window.once('focus', () => { flashing = false; window.flashFrame(false); });
    }
    if (Notification.isSupported()) {
      shown = new Notification({ title, body: said(message?.body, 240) });
      shown.on('click', () => {
        if (window.isMinimized()) window.restore();
        window.show();
        window.focus();
      });
      shown.show();
    }
    return true;
  });
  ipcMain.handle('columbina:services', async (event) => {
    checkSender(event);
    if (!programCommand([])) return null;
    // The document says which services there are and where their keys come from. It holds no key.
    const document = await runProgram(['setup', '--list']);
    if (typeof document.error === 'string') throw new Error(document.error);
    mainListed = namesAndPages(document).names;
    return document;
  });
  ipcMain.handle('columbina:models', async (event, name) => {
    checkSender(event);
    // Only a service that the window was told of. The document has no key: the program asks with
    // the one that a session would send, so no key comes this way.
    if (typeof name !== 'string' || !mainListed.includes(name)) throw new Error('That service is not among those that are set up.');
    return runProgram(['setup', '--check'], JSON.stringify({ service: name }));
  });
  ipcMain.handle('columbina:reasoning-language', async (event, language) => {
    checkSender(event);
    if (!['zh', 'en', 'ja', 'none'].includes(language)) throw new Error('Unknown language of reasoning.');
    // A preference that the page keeps in step by itself: without a program (a replay, or
    // development without a service), there is nothing to tell, and nothing went wrong. The page
    // tells it again once there is a program.
    if (!programCommand([])) return { skipped: true };
    const said = await runProgram(['reasoning-language', language, '--json']);
    if (typeof said.error === 'string') throw new Error(said.error);
    return said;
  });
  ipcMain.handle('columbina:balance', async (event, name) => {
    checkSender(event);
    // As for the models: a service that the window was told of, asked with its own key.
    if (typeof name !== 'string' || !mainListed.includes(name)) throw new Error('That service is not among those that are set up.');
    return runProgram(['balance', '--json', '--service', name]);
  });
  ipcMain.handle('columbina:choose-model', async (event, asked) => {
    checkSender(event);
    const name = asked?.service;
    const model = asked?.model;
    if (typeof name !== 'string' || !mainListed.includes(name)) throw new Error('That service is not among those that are set up.');
    // A name as a service gives one: of one piece, and not long. What looks like a key is none:
    // a model is kept as it is written, and not in the keyring. The program checks the rest.
    if (typeof model !== 'string' || !/^\S{1,200}$/.test(model) || /[\x00-\x1f\x7f]/.test(model)
      || /^sk-/i.test(model) || /^[A-Za-z0-9_]{40,}$/.test(model)) throw new Error('That is no name of a model.');
    // Without a key, the key that is stored for the service stays as it is.
    return inTurn(() => runProgram(['setup', '--from-stdin'], JSON.stringify({ service: name, model })));
  });
  ipcMain.handle('columbina:choose-effort', async (event, asked) => {
    checkSender(event);
    const name = asked?.service;
    const effort = asked?.effort;
    if (typeof name !== 'string' || !mainListed.includes(name)) throw new Error('That service is not among those that are set up.');
    // A level that is kept for the next start. Ultra holds for one run, and is not kept.
    if (!['default', 'off', 'low', 'high', 'max'].includes(effort)) throw new Error('That is no level of thinking that is kept.');
    // A document of the service and the level alone changes the level, and nothing else.
    return inTurn(() => runProgram(['setup', '--from-stdin'], JSON.stringify({ service: name, effort })));
  });
  ipcMain.handle('columbina:choose-attachments', async (event, asked) => {
    checkSender(event);
    const title = WORDS[spoken(asked?.language)].chooseFiles;
    const result = await dialog.showOpenDialog(window, { title, properties: ['openFile', 'multiSelections'] });
    if (result.canceled) return null;
    return attach(result.filePaths);
  });
  ipcMain.handle('columbina:attach-paths', async (event, paths) => {
    checkSender(event);
    // The paths of files that were dropped on the window, as the preload has them from the files
    // themselves. The page has no way of its own to name a path here.
    if (!Array.isArray(paths) || paths.length > ATTACHMENTS_HELD_MOST
      || paths.some((each) => typeof each !== 'string' || !path.isAbsolute(each))) throw new Error('Those are no files that were dropped.');
    return attach(paths);
  });
  ipcMain.handle('columbina:forget-attachments', (event, ids) => {
    checkSender(event);
    for (const id of Array.isArray(ids) ? ids : []) attachments.delete(id);
  });
  ipcMain.handle('columbina:keep-attachments', async (event, ids) => {
    checkSender(event);
    if (!Array.isArray(ids) || ids.length === 0 || ids.length > ATTACHMENTS_MOST) throw new Error('That is no set of files for a message.');
    const chosen = ids.map((id) => (typeof id === 'string' ? attachments.get(id) : undefined));
    if (chosen.includes(undefined)) throw new Error('A file that is to go with the message was not chosen.');
    const project = projectOfTheService();
    if (project === null) throw new Error('No project is open, so the files have nowhere to be kept.');
    // Each file is as it was chosen, or none is copied.
    for (const file of chosen) {
      const found = await stat(file.path).catch(() => null);
      if (found === null) throw new Error(`${file.name} is not there any more.`);
      if (!found.isFile() || found.size > ATTACHMENT_MOST_BYTES) throw new Error(`${file.name} is no longer a file that can go with a message.`);
    }
    const root = path.join(project, ...ATTACHMENTS_FOLDER);
    const folder = path.join(root, sentAt(new Date()));
    await mkdir(folder, { recursive: true });
    // What is kept for the messages stays out of the history of the project.
    await writeFile(path.join(root, '.gitignore'), '*\n', { flag: 'wx' }).catch((error) => {
      if (error?.code !== 'EEXIST') throw error;
    });
    const kept = [];
    for (const file of chosen) kept.push([...ATTACHMENTS_FOLDER, path.basename(folder), await copiedInto(folder, file)].join('/'));
    // The files are still held until the page lets go of them: a message that does not go is sent
    // again with them.
    return { kept };
  });
  ipcMain.handle('columbina:attach-pasted', async (event, pasted) => {
    checkSender(event);
    // Pictures that were pasted have no file. Each is written into a folder of the shell's own,
    // and held from there as a chosen file is.
    if (!Array.isArray(pasted) || pasted.length === 0 || pasted.length > ATTACHMENTS_MOST) throw new Error('Those are no pictures that were pasted.');
    const folder = path.join(app.getPath('temp'), 'columbina-pasted');
    await mkdir(folder, { recursive: true });
    const paths = [];
    for (const one of pasted) {
      const ending = PASTED_KINDS[one?.type];
      const bytes = one?.bytes;
      // Bytes that came over a channel are an ArrayBuffer of another realm, which instanceof does not know.
      const buffer = Object.prototype.toString.call(bytes) === '[object ArrayBuffer]' ? bytes : null;
      if (ending === undefined || buffer === null || buffer.byteLength === 0
        || buffer.byteLength > ATTACHMENT_MOST_BYTES) throw new Error('A pasted picture is of no kind that is taken, or of no size that is.');
      const written = path.join(folder, `pasted-${sentAt(new Date())}-${++pastedNumber}.${ending}`);
      await writeFile(written, new Uint8Array(buffer));
      paths.push(written);
    }
    return attach(paths);
  });
  ipcMain.handle('columbina:schedules', async (event, asked) => {
    checkSender(event);
    const what = asked?.what;
    const words = ['schedules'];
    if (what === 'list') words.push('list');
    else if (what === 'add') {
      const task = asked?.task ?? {};
      // A task runs in a folder that the user chose in the dialog, as a project does.
      const cwd = approvedDirectory({ cwd: task.cwd });
      const line = (value) => (typeof value === 'string' && value.trim() !== '' && !/[\u0000-\u0008\u000b-\u001f]/.test(value) ? value : null);
      const name = line(task.name);
      const prompt = line(task.prompt);
      if (name === null || prompt === null || !['minutes', 'hour', 'day', 'week'].includes(task.every)) throw new Error('That is no task that can run at set times.');
      // Every value is given with its option in one word, so that none is read as an option itself.
      words.push('add', `--name=${name}`, `--cwd=${cwd}`, `--prompt=${prompt}`, `--every=${task.every}`);
      if (Number.isInteger(task.minutes)) words.push(`--minutes=${task.minutes}`);
      if (typeof task.at === 'string' && /^\d{0,2}:\d{2}$/.test(task.at)) words.push(`--at=${task.at}`);
      if (['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'].includes(task.weekday)) words.push(`--weekday=${task.weekday}`);
      if (['run', 'skip'].includes(task.missed)) words.push(`--missed=${task.missed}`);
    } else if (['remove', 'enable', 'disable'].includes(what) && typeof asked?.id === 'string' && /^[0-9a-z][\w-]{0,63}$/i.test(asked.id)) {
      words.push(what, asked.id);
    } else throw new Error('That is nothing that can be done with the tasks that run at set times.');
    const said = await runProgram([...words, '--json']);
    if (typeof said.error === 'string') throw new Error(said.error);
    return said;
  });
  ipcMain.handle('columbina:worktrees', async (event, asked) => {
    checkSender(event);
    const cwd = approvedDirectory({ cwd: asked?.cwd });
    const what = asked?.what;
    const words = ['worktrees', what, `--cwd=${cwd}`, '--json'];
    if (!['list', 'add', 'merge', 'remove'].includes(what)) throw new Error('That is nothing that can be done with worktrees.');
    if (what !== 'list') {
      const name = typeof asked?.name === 'string' ? asked.name.trim() : '';
      if (name === '' || name.length > 100 || /[\u0000-\u001f]/.test(name)) throw new Error('That is no name of a task.');
      if (what === 'remove' && asked?.force === true) words.push('--force');
      words.push('--', name);
    }
    const said = await runProgram(words);
    if (typeof said.error === 'string') throw new Error(said.error);
    // A worktree lies in a project that the user chose, and may be opened as one.
    const made = what === 'list' ? (Array.isArray(said.worktrees) ? said.worktrees : []) : what === 'add' ? [said] : [];
    for (const one of made) {
      if (typeof one?.path !== 'string') continue;
      try { approvedDirectories.add(directoryKey(realpathSync(one.path))); } catch { /* It is gone already. */ }
    }
    return said;
  });
  ipcMain.handle('columbina:qq-groups', async (event, asked) => {
    checkSender(event);
    const what = asked?.what;
    // An id begins with a letter or a digit, so that it is never read as an option.
    const group = typeof asked?.id === 'string' && /^[0-9a-z][\w-]{0,127}$/i.test(asked.id) ? asked.id : null;
    let words;
    if (what === 'list') words = ['qq', '--groups'];
    else if (group === null) throw new Error('That is no group of QQ.');
    else if (what === 'name') {
      const name = typeof asked?.name === 'string' && !/[\u0000-\u001f]/.test(asked.name) ? asked.name.trim() : null;
      // A name that begins with a dash would be read as an option. No name is a blank, which takes it away.
      if (name === null || name.length > 64 || name.startsWith('-')) throw new Error('That is no name of a group.');
      words = ['qq', '--name-group', group, name === '' ? ' ' : name];
    } else if (what === 'memory') words = ['qq', `--memory=${group}`];
    else if (what === 'forget') words = ['qq', `--forget-memory=${group}`];
    else throw new Error('That is nothing that can be done with the groups of QQ.');
    const said = await runProgram([...words, '--json']);
    if (typeof said.error === 'string') throw new Error(said.error);
    return said;
  });
  ipcMain.handle('columbina:set-up-service', async (event, asked) => {
    checkSender(event);
    if (!programCommand([])) throw new Error(NO_SETUP);
    if (keyWindow) {
      keyWindow.focus();
      return keyClosed;
    }
    keyAsked = {
      service: typeof asked?.service === 'string' && asked.service ? asked.service : null,
      language: spoken(asked?.language),
    };
    keyChanged = false;
    keyListed = { names: [], pages: [] };
    // A window of its own, so that the key is never in the window that shows what a model wrote.
    const opened = new BrowserWindow({
      parent: window, modal: true, width: 540, height: 680, resizable: false, minimizable: false, maximizable: false,
      show: false, backgroundColor: titleBarThemes[preferences.theme].color, title: 'Columbina',
      icon: path.join(__dirname, 'icon.ico'),
      webPreferences: { preload: path.join(__dirname, 'key-preload.cjs'), contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true },
    });
    keyWindow = opened;
    opened.setMenuBarVisibility(false);
    closeToTheWorld(opened);
    opened.once('ready-to-show', () => opened.show());
    keyClosed = new Promise((resolve) => {
      opened.once('closed', () => {
        if (keyWindow === opened) keyWindow = null;
        // The main window is told that something was stored, and nothing of what.
        resolve({ changed: keyChanged });
      });
    });
    void opened.loadFile(keyPath);
    return keyClosed;
  });
  ipcMain.handle('columbina-key:context', async (event) => {
    checkKeySender(event);
    const list = await runProgram(['setup', '--list']);
    keyListed = namesAndPages(list);
    return { ...keyAsked, theme: preferences.theme, list };
  });
  ipcMain.handle('columbina-key:check', async (event, given) => {
    checkKeySender(event);
    // What the program reads to check a service, each of its kind. A model and a size are not checked.
    const document = {};
    for (const name of ['service', 'base_url', 'api_key']) {
      if (typeof given?.[name] === 'string') document[name] = given[name];
    }
    return runProgram(['setup', '--check'], JSON.stringify(document));
  });
  ipcMain.handle('columbina-key:forget', async (event, name) => {
    checkKeySender(event);
    // Only a service that the window was told of: the name goes on a command line.
    if (typeof name !== 'string' || !keyListed.names.includes(name)) throw new Error('That service is not among those that are set up.');
    const answer = await inTurn(() => runProgram(['setup', '--forget', name, '--json']));
    if (answer.forgotten) keyChanged = true;
    return answer;
  });
  ipcMain.handle('columbina-key:open-page', async (event, address) => {
    checkKeySender(event);
    // Only a page that a service links to, as the program names it, and over https.
    if (typeof address !== 'string' || !keyListed.pages.includes(address)) throw new Error('That page is none that a service links to.');
    const url = new URL(address);
    if (url.protocol !== 'https:') throw new Error('Only pages over https are opened.');
    await shell.openExternal(url.href);
  });
  ipcMain.handle('columbina-key:store', async (event, given) => {
    checkKeySender(event);
    // The five things that the program reads, each of its kind. The program checks the rest.
    const document = {};
    for (const name of ['service', 'base_url', 'model', 'api_key']) {
      if (typeof given?.[name] === 'string') document[name] = given[name];
    }
    if (Number.isInteger(given?.context_window)) document.context_window = given.context_window;
    const answer = await inTurn(() => runProgram(['setup', '--from-stdin'], JSON.stringify(document)));
    if (answer.stored) keyChanged = true;
    return answer;
  });
  ipcMain.handle('columbina-key:close', (event) => {
    checkKeySender(event);
    keyWindow.close();
  });
  ipcMain.handle('columbina:open-external', async (event, address) => {
    checkSender(event);
    if (typeof address !== 'string') throw new Error('The link address must be a string.');
    const url = new URL(address);
    if (url.protocol !== 'https:' && url.protocol !== 'http:') throw new Error('Only http and https links can be opened.');
    await shell.openExternal(url.href);
  });
  ipcMain.handle('columbina:about', (event) => {
    checkSender(event);
    // The version that runs, and whether this application asks for newer ones at all.
    return { version: app.getVersion(), checks: releasesFeed() !== null };
  });
  ipcMain.handle('columbina:updates', async (event, asked) => {
    checkSender(event);
    const feed = releasesFeed();
    if (feed === null) throw new Error('This application does not look for newer versions.');
    // An answer is taken as it is for an hour, so that a page that is loaded again does not ask
    // again. Asked now, GitHub is asked now. One question is under way at a time.
    if (asked?.now !== true && updateKnown !== null && Date.now() - updateKnown.at < UPDATE_KEPT_MS) return updateKnown.answer;
    updateAsking ??= newerRelease(feed).then((answer) => {
      // What failed is asked again the next time.
      updateKnown = answer.failure === null ? { at: Date.now(), answer } : null;
      return answer;
    }).finally(() => { updateAsking = null; });
    return updateAsking;
  });
  ipcMain.handle('columbina:github-status', async (event) => {
    checkSender(event);
    if (!programCommand([])) return null;
    // Who is signed in, the picture as an address of data, and what Columbina may reach. No token.
    const answer = await runProgram(['login', '--status', '--json'], '', ACCOUNT_OUTPUT_MOST);
    return { ...answer, skipped: preferences.githubSkipped };
  });
  ipcMain.handle('columbina:github-sign-in', async (event) => {
    checkSender(event);
    if (askingLoginShell) await askingLoginShell;
    const command = programCommand(['login', 'github', '--json']);
    if (!command) throw new Error(NO_SETUP);
    // One sign-in at a time, and the page hears the one that runs. One that is begun takes the
    // place of one that runs: nobody waits for that one any more, as after the page was loaded again.
    stopSigningIn();
    const child = spawn(command.executable, command.args, {
      cwd: app.getPath('home'), env: command.env, windowsHide: true,
      detached: process.platform !== 'win32', stdio: ['pipe', 'pipe', 'pipe'],
    });
    signingIn = child;
    const lines = createInterface({ input: child.stdout, crlfDelay: Infinity });
    lines.on('line', (line) => {
      // Each line is a document of JSON: the code first, how the sign-in ended last. No line holds a token.
      if (signingIn !== child || line.length > ACCOUNT_OUTPUT_MOST) return;
      let said = null;
      try { said = JSON.parse(line); } catch { return; }
      if (said !== null && typeof said === 'object' && !Array.isArray(said)) tell('columbina:github', said);
    });
    // What a person would be shown. It is taken so that the program is not held up, and kept nowhere.
    child.stderr.on('data', () => {});
    child.stdin.on('error', () => {});
    child.on('close', () => {
      lines.close();
      if (signingIn !== child) return;
      signingIn = null;
      tell('columbina:github', { ended: true });
    });
    let started = false;
    await new Promise((resolve, reject) => {
      child.once('spawn', () => { started = true; resolve(); });
      child.on('error', (error) => {
        // A program that started ends by itself, and its end is told of. One that did not never will.
        if (started) return;
        if (signingIn === child) signingIn = null;
        reject(new Error(`The sign-in could not be started: ${error.message}`));
      });
    });
  });
  ipcMain.handle('columbina:github-cancel', (event) => {
    checkSender(event);
    stopSigningIn();
  });
  ipcMain.handle('columbina:github-sign-out', async (event) => {
    checkSender(event);
    return runProgram(['logout', 'github', '--json']);
  });
  ipcMain.handle('columbina:skip-github', (event, skipped) => {
    checkSender(event);
    preferences.githubSkipped = skipped === true;
    keepPreferences();
  });
  ipcMain.handle('columbina:engine-open', async (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    // A project of the studio lies in a folder that the user chose in the dialog, as a project does.
    // Checked before its turn, so that a request that is refused closes nothing that is open.
    approvedDirectory(asked, 'opening it in the studio');
    return inStudioTurn(() => openStudio(approvedDirectory(asked, 'opening it in the studio')));
  });
  ipcMain.handle('columbina:engine-call', async (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    const method = asked?.method;
    if (typeof method !== 'string' || !PAGE_METHODS.has(method)) throw new Error('That is no method of the engine that the page may call.');
    const params = asked?.params ?? null;
    if (params !== null && (typeof params !== 'object' || Array.isArray(params))) throw new Error('What a method of the engine is given is an object.');
    if (method === 'transaction.apply') transactionOfThePage(params);
    if (method === 'viewer.wipe' || method === 'viewer.split') comparisonOfThePage(params);
    // The page cancels the exports that it started, not those of an agent on the same engine.
    if (method === 'render.cancel' && !(typeof params?.job === 'string' && studio?.jobs.has(params.job))) {
      throw new Error('The page cancels only an export that it started.');
    }
    if (method.startsWith('transport.')) monitorOf(params?.monitor);
    const open = studio;
    if (!open) return { ok: false, error: { code: null, message: 'No project is open in the studio.', data: null } };
    if (asked?.expectedSession !== undefined && asked.expectedSession !== open.number) {
      return { ok: false, error: { code: null, message: 'The project of this request is no longer open in the studio.', data: { reason: 'stale_session' } } };
    }
    // The viewer goes through the shell, which remembers it for the next view that it makes.
    if (method === 'view.place') return answered(Promise.resolve().then(() => { takeViewRect(params); return {}; }));
    if (method === 'view.show') return answered(Promise.resolve().then(() => { takeViewShown(params); return {}; }));
    if (method === 'scopes.subscribe' || method === 'scopes.unsubscribe') return answered(keepScopes(open, method, params));
    if (method === 'viewer.preview' && params?.on !== false) open.previewed = true;
    // Statistics can compile and draw for longer than the engine's gesture idle timeout.
    // Keep the main connection free for edits/gesture keep-alives while that draw runs. Other
    // requests of the view connection wait behind the draw; both connections belong to `open`.
    if (method === 'scopes.statistics') return answered(open.session.view(method, params));
    return answered(open.session.call(method, params));
  });
  // The floating window of the scopes, which the page opens, tells what it measures, and closes.
  ipcMain.handle('columbina:engine-scopes-window', (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    if (asked?.open === true) openScopesWindow(asked); else closeScopesWindow();
    return { open: scopesWindow !== null };
  });
  ipcMain.handle('columbina-scopes:context', (event) => {
    checkScopesSender(event);
    return scopesContext();
  });
  ipcMain.handle('columbina-scopes:call', async (event, asked) => {
    checkScopesSender(event);
    const method = asked?.method;
    if (typeof method !== 'string' || !SCOPE_WINDOW_METHODS.has(method)) throw new Error('That is no method of the engine that the window of the scopes may call.');
    const params = asked?.params ?? null;
    if (params === null || typeof params !== 'object' || Array.isArray(params)) throw new Error('What a method of the engine is given is an object.');
    const open = studio;
    if (!open) return { ok: false, error: { code: null, message: 'No project is open in the studio.', data: null } };
    // Its own scopes alone: another page's are not its to set or let go.
    if (method !== 'scopes.subscribe' && !open.windowScopes.has(params.scope_id)) {
      return { ok: false, error: { code: null, message: 'That scope is not one of this window.', data: null } };
    }
    const asker = scopesWindow;
    const round = windowScopesRound;
    return answered(open.session.call(method, params).then((result) => {
      if (method === 'scopes.subscribe' && typeof result?.scope_id === 'string') {
        // A window that was closed or loaded again meanwhile let its scopes go: this one goes too,
        // or its pictures would go to no one and the next window would hold it.
        if (scopesWindow !== asker || !asker || asker.isDestroyed() || windowScopesRound !== round || studio !== open) {
          if (open.session.alive) void open.session.call('scopes.unsubscribe', { scope_id: result.scope_id }).catch(() => {});
        } else open.windowScopes.add(result.scope_id);
      }
      if (method === 'scopes.unsubscribe') open.windowScopes.delete(params.scope_id);
      return result;
    }));
  });
  ipcMain.handle('columbina-scopes:close', (event) => {
    checkScopesSender(event);
    closeScopesWindow();
  });
  ipcMain.handle('columbina:engine-import', async (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    const open = studio;
    if (!open) return { ok: false, error: { code: null, message: 'No project is open in the studio.', data: null } };
    const bin = asked?.bin;
    if (bin !== undefined && (typeof bin !== 'string' || !bin)) throw new Error('Name a bin by its id.');
    // The engine is given only files that the user chose in the dialog of the system.
    const words = WORDS[spoken(asked?.language)];
    const result = await dialog.showOpenDialog(window, {
      title: words.chooseMedia, properties: ['openFile', 'multiSelections'],
      filters: [{ name: words.media, extensions: MEDIA_ENDINGS }, { name: words.allFiles, extensions: ['*'] }],
    });
    const paths = result.canceled ? [] : result.filePaths.filter((chosen) => typeof chosen === 'string' && path.isAbsolute(chosen));
    if (paths.length === 0) return { ok: true, result: null };
    if (studio !== open) return { ok: false, error: { code: null, message: 'The project of the studio was closed meanwhile.', data: null } };
    return answered(open.session.call('media.import', { paths, ...(bin === undefined ? {} : { bin }) }));
  });
  ipcMain.handle('columbina:engine-still-import', async (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    const open = studio;
    if (!open) return { ok: false, error: { code: null, message: 'No project is open in the studio.', data: null } };
    const album = asked?.album;
    if (album !== undefined && (typeof album !== 'string' || !album)) throw new Error('Name an album by its id.');
    // Import (Reference Manual 20, p. 3270): the engine is given only pictures that the user chose in the dialog.
    const words = WORDS[spoken(asked?.language)];
    const result = await dialog.showOpenDialog(window, {
      title: words.importStills, properties: ['openFile', 'multiSelections'],
      filters: [{ name: words.stills, extensions: STILL_ENDINGS }, { name: words.allFiles, extensions: ['*'] }],
    });
    const paths = result.canceled ? [] : result.filePaths.filter((chosen) => typeof chosen === 'string' && path.isAbsolute(chosen));
    if (paths.length === 0) return { ok: true, result: null };
    if (studio !== open) return { ok: false, error: { code: null, message: 'The project of the studio was closed meanwhile.', data: null } };
    return answered(open.session.call('still.import', { paths, ...(album === undefined ? {} : { album }) }));
  });
  ipcMain.handle('columbina:engine-still-export', async (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    const open = studio;
    if (!open) return { ok: false, error: { code: null, message: 'No project is open in the studio.', data: null } };
    const stills = asked?.stills;
    if (!Array.isArray(stills) || stills.length === 0 || !stills.every(still => typeof still === 'string' && still)) {
      throw new Error('Name the stills to export by their ids.');
    }
    // Export (p. 3271): a place is chosen and a name typed in Save As; each still is written there
    // as that name, an underscore and its number. Files that are there are not replaced.
    const words = WORDS[spoken(asked?.language)];
    const folder = path.resolve(open.project, ...STUDIO_FOLDER.map(() => '..'));
    const chosen = await dialog.showSaveDialog(window, {
      title: words.exportStills, defaultPath: path.join(folder, 'Still'),
      filters: [{ name: words.stills, extensions: ['png', 'exr'] }],
    });
    if (chosen.canceled) return { ok: true, result: null };
    if (typeof chosen.filePath !== 'string' || !path.isAbsolute(chosen.filePath)) throw new Error('Choose a whole path in Save As.');
    if (studio !== open) return { ok: false, error: { code: null, message: 'The project of the studio was closed meanwhile.', data: null } };
    const prefix = path.basename(chosen.filePath, path.extname(chosen.filePath));
    return answered(open.session.call('still.export', { stills: [...stills], dir: path.dirname(chosen.filePath), ...(prefix ? { prefix } : {}) }));
  });
  ipcMain.handle('columbina:engine-export-lut', async (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    const open = studio;
    if (!open) return { ok: false, error: { code: null, message: 'No project is open in the studio.', data: null } };
    // Generate LUT (Reference Manual 20, p. 3413): a grade the page names by its target, baked
    // into a cube of 17, 33 or 65 points, written where Save As says and nowhere else.
    const grade = asked?.grade;
    const keys = ['level', 'timeline', 'clip', 'group'];
    if (typeof grade !== 'object' || grade === null || Array.isArray(grade) || typeof grade.level !== 'string'
      || !Object.entries(grade).every(([key, value]) => keys.includes(key) && typeof value === 'string' && value)) {
      throw new Error('Name the grade by its level and where it is.');
    }
    if (!LUT_SIZES.includes(asked?.size)) throw new Error('A cube is of 17, 33 or 65 points.');
    const node = asked?.node;
    if (node !== undefined && (typeof node !== 'string' || !node)) throw new Error('Name a node by its id.');
    const frame = asked?.frame;
    if (frame !== undefined && !Number.isSafeInteger(frame)) throw new Error('A frame is a whole number.');
    const words = WORDS[spoken(asked?.language)];
    // Save As opens in the library of LUTs, as DaVinci Resolve's opens in its LUT folder, so that
    // the cube saved there is listed and can be put on a node (p. 3413); else in the project's folder.
    const listed = await answered(open.session.call('luts.list', null));
    const roots = listed.ok && Array.isArray(listed.result?.roots) ? listed.result.roots : [];
    const library = roots.find(each => each?.place === 'library' && each.exists === true && typeof each.path === 'string' && path.isAbsolute(each.path));
    const folder = library ? library.path : path.resolve(open.project, ...STUDIO_FOLDER.map(() => '..'));
    if (studio !== open) return { ok: false, error: { code: null, message: 'The project of the studio was closed meanwhile.', data: null } };
    const chosen = await dialog.showSaveDialog(window, {
      title: words.generateLut, defaultPath: path.join(folder, `${fileNameOf(asked?.name)}.cube`),
      filters: [{ name: words.cube, extensions: ['cube'] }],
      // Windows and macOS ask before a file is replaced; Linux only where it is told to.
      properties: ['showOverwriteConfirmation'],
    });
    if (chosen.canceled) return { ok: true, result: null };
    if (typeof chosen.filePath !== 'string' || !path.isAbsolute(chosen.filePath)) throw new Error('Choose a whole path in Save As.');
    if (studio !== open) return { ok: false, error: { code: null, message: 'The project of the studio was closed meanwhile.', data: null } };
    // Save As asked before a file there is replaced; the engine replaces only a file.
    return answered(open.session.call('grade.export_lut', {
      grade: { ...grade }, size: asked.size, path: chosen.filePath, replace: true,
      ...(node === undefined ? {} : { node }), ...(frame === undefined ? {} : { frame }),
    }));
  });
  ipcMain.handle('columbina:engine-render', async (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    const open = studio;
    if (!open) return { ok: false, error: { code: null, message: 'No project is open in the studio.', data: null } };
    const options = renderOptionsOf(asked);
    const mov = options.codec === 'prores';
    const words = WORDS[spoken(asked?.language)];
    // The dialog opens in the folder of the project, with the file named after the timeline.
    const folder = path.resolve(open.project, ...STUDIO_FOLDER.map(() => '..'));
    const chosen = await dialog.showSaveDialog(window, {
      title: words.chooseExport, defaultPath: path.join(folder, `${fileNameOf(asked?.name)}${mov ? '.mov' : '.mp4'}`),
      filters: [{ name: mov ? 'MOV' : 'MP4 / MOV', extensions: mov ? ['mov'] : ['mp4', 'mov'] }],
    });
    if (chosen.canceled) return { ok: true, result: { cancelled: true } };
    if (studio !== open) return { ok: false, error: { code: null, message: 'The project of the studio was closed meanwhile.', data: null } };
    if (typeof chosen.filePath !== 'string' || !path.isAbsolute(chosen.filePath)) throw new Error('Choose a whole output path in Save As.');
    // The engine checks its output paths (including fallback MOV and SRT) and refuses an existing
    // one, unless it may replace them. Only the shell decides that, from what the user confirmed
    // in Save As (see `replacing`); the page never can.
    const closed = { ok: false, error: { code: null, message: 'The project of the studio was closed meanwhile.', data: null } };
    for (let tries = 1; ; tries += 1) {
      const there = await besideOf(chosen.filePath);
      if (studio !== open) return closed;
      // The names of the SRT files are asked for last, at a revision of the project that render.start
      // is held to: where another client renamed a subtitle track meanwhile, the engine refuses it as
      // stale_revision before it writes or replaces anything, and the export is prepared again.
      const { names: srts, revision } = there === null ? { names: [], revision: null } : await srtFilesOf(open, chosen.filePath, options);
      if (studio !== open) return closed;
      const { replace, blocking } = replacing(chosen.filePath, options, there, srts);
      // A file beside the film that Save As did not ask of: refused naming that file, not the film
      // that the user just confirmed, as the engine would refuse it.
      if (blocking !== null) {
        const reason = blocking.toLowerCase().endsWith('.part') ? 'part_exists' : 'exists';
        return { ok: false, error: { code: -32001, message: `${blocking} is there already.`, data: { reason, path: blocking } } };
      }
      const started = await answered(open.session.call('render.start', { ...options, path: chosen.filePath, replace, ...(revision === null ? {} : { revision }) }));
      if (!started.ok && started.error.data?.reason === 'stale_revision' && revision !== null && tries < RENDER_TRIES && studio === open) continue;
      if (started.ok && typeof started.result?.job === 'string' && !open.jobs.has(started.result.job)) open.jobs.set(started.result.job, { path: chosen.filePath });
      return started;
    }
  });
  ipcMain.handle('columbina:engine-reveal', async (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    // Only the file of an export that this window started, where the engine said it wrote it: the page names no path.
    const job = typeof asked?.job === 'string' ? studio?.jobs.get(asked.job) : undefined;
    if (!job || !existsSync(job.path)) return false;
    shell.showItemInFolder(job.path);
    return true;
  });
  ipcMain.handle('columbina:engine-open-lut-folder', async (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    // Open LUT Folder (Reference Manual 20, p. 3409): only a folder that the engine lists as a root
    // of LUTs now (`luts.list`), the one the page names among them, never another path.
    const open = studio;
    const named = asked?.path;
    if (!open || typeof named !== 'string' || !path.isAbsolute(named)) return false;
    const listed = await answered(open.session.call('luts.list', null));
    const roots = listed.ok && Array.isArray(listed.result?.roots) ? listed.result.roots : [];
    const root = roots.find(each => typeof each?.path === 'string' && path.isAbsolute(each.path) && each.path === named);
    if (!root || studio !== open) return false;
    try {
      if (!statSync(root.path).isDirectory()) return false;
    } catch {
      return false;
    }
    const failed = await shell.openPath(root.path);
    if (failed) throw new Error(failed);
    return true;
  });
  ipcMain.handle('columbina:left-key', (event, asked) => {
    checkSender(event);
    // macOS keeps its menu, whose keys never come to the page.
    if (process.platform === 'darwin') return false;
    return pageLeftKey(asked);
  });
  ipcMain.handle('columbina:engine-view-rect', (event, rect) => {
    checkSender(event);
    refuseStudioElsewhere();
    takeViewRect(rect);
  });
  ipcMain.handle('columbina:engine-view-show', (event, asked) => {
    checkSender(event);
    refuseStudioElsewhere();
    takeViewShown(asked);
  });
  ipcMain.handle('columbina:engine-close', async (event) => {
    checkSender(event);
    await inStudioTurn(closeStudio);
  });
  for (const one of Object.values(assistants)) installAssistant(one);
}
/** Answer the main window about an assistant, and the window in which it is set up. */
function installAssistant(one) {
  const { kind } = one;
  ipcMain.handle(`columbina:${kind}-status`, async (event) => {
    checkSender(event);
    if (!programCommand([])) return null;
    // What is set up and how its assistant is doing, without the secret; whether this application
    // runs the assistant and starts it with itself; and how the last one ended badly.
    const answer = await runProgram([kind, '--status', '--json']);
    return { ...answer, own: one.child !== null, on: preferences[`${kind}On`], ended: one.ended };
  });
  ipcMain.handle(`columbina:${kind}-start`, async (event, asked) => {
    checkSender(event);
    // It speaks the language of the page, also when it starts again with the application.
    keepAssistantLanguage(one, assistantLanguageOf(asked?.language));
    await startAssistant(one);
    // It runs again when the application is started again, until the user stops it.
    keepAssistantOn(one, true);
  });
  ipcMain.handle(`columbina:${kind}-stop`, async (event) => {
    checkSender(event);
    keepAssistantOn(one, false);
    await stopAssistant(one);
  });
  ipcMain.handle(`columbina:${kind}-forget`, async (event) => {
    checkSender(event);
    // An assistant goes on with what it read when it started, so it is stopped first.
    keepAssistantOn(one, false);
    await stopAssistant(one);
    return inTurn(() => runProgram([kind, '--forget', '--json']));
  });
  ipcMain.handle(`columbina:${kind}-log`, (event) => {
    checkSender(event);
    shell.showItemInFolder(assistantLogPath(one));
  });
  if (one.writable) ipcMain.handle(`columbina:${kind}-writable`, async (event, writable) => {
    checkSender(event);
    if (typeof writable !== 'boolean') throw new Error('Whether the assistant may change files is true or false.');
    // The program changes it only while no assistant runs. One that the application runs is
    // stopped first and started again after, with what was changed.
    const ran = one.child !== null;
    if (ran) await stopAssistant(one);
    let answer;
    try {
      answer = await inTurn(() => runProgram([kind, '--from-stdin'], JSON.stringify({ writable })));
    } finally {
      if (ran && !exiting) await startAssistant(one).catch(() => {});
    }
    return answer;
  });
  ipcMain.handle(`columbina:${kind}-thinking`, async (event, asked) => {
    checkSender(event);
    const model = asked?.model ?? null;
    const effort = asked?.effort ?? null;
    if (model !== null && (typeof model !== 'string' || !model.trim() || model.length > 200)) {
      throw new Error('A model is named by its id, as the service lists it.');
    }
    if (effort !== null && !EFFORTS.includes(effort)) throw new Error('How hard the model thinks is one of default, off, low, high and max.');
    // As for whether it may change files: the program changes it only while no assistant runs.
    const ran = one.child !== null;
    if (ran) await stopAssistant(one);
    let answer;
    try {
      const chosen = JSON.stringify({ model: model === null ? null : model.trim(), effort });
      answer = await inTurn(() => runProgram([kind, '--from-stdin'], chosen));
    } finally {
      if (ran && !exiting) await startAssistant(one).catch(() => {});
    }
    return answer;
  });
  ipcMain.handle(`columbina:${kind}-folder`, async (event) => {
    checkSender(event);
    // The folder that the program names, and no other: the page names none.
    const folder = (await runProgram([kind, '--status', '--json']))?.folder;
    if (typeof folder !== 'string' || !path.isAbsolute(folder)) throw new Error('The program named no folder of the assistant.');
    mkdirSync(folder, { recursive: true });
    const failed = await shell.openPath(folder);
    if (failed) throw new Error(failed);
  });
  ipcMain.handle(`columbina:set-up-${kind}`, async (event, asked) => {
    checkSender(event);
    if (!programCommand([])) throw new Error(NO_SETUP);
    if (one.window) {
      one.window.focus();
      return one.closed;
    }
    one.asked = { language: spoken(asked?.language) };
    one.changed = false;
    // A window of its own, so that the secret is never in the window that shows what a model wrote.
    const opened = new BrowserWindow({
      parent: window, modal: true, width: 560, height: 720, resizable: false, minimizable: false, maximizable: false,
      show: false, backgroundColor: titleBarThemes[preferences.theme].color, title: 'Columbina',
      icon: path.join(__dirname, 'icon.ico'),
      webPreferences: { preload: path.join(__dirname, `${kind}-preload.cjs`), contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true },
    });
    one.window = opened;
    opened.setMenuBarVisibility(false);
    closeToTheWorld(opened);
    opened.on('close', (closing) => { if (one.storing) closing.preventDefault(); });
    opened.once('ready-to-show', () => opened.show());
    one.closed = new Promise((resolve) => {
      opened.once('closed', () => {
        if (one.window === opened) one.window = null;
        // The main window is told that something was stored, and nothing of it.
        resolve({ changed: one.changed });
      });
    });
    void opened.loadFile(one.page);
    return one.closed;
  });
  ipcMain.handle(`columbina-${kind}:context`, async (event) => {
    checkAssistantSender(event, one);
    const status = await runProgram([kind, '--status', '--json']);
    return { ...one.asked, theme: preferences.theme, status };
  });
  ipcMain.handle(`columbina-${kind}:store`, async (event, given) => {
    checkAssistantSender(event, one);
    // The things that the program reads, each of its kind. The program checks the rest.
    const document = {};
    for (const name of one.fields) {
      if (typeof given?.[name] === 'string') document[name] = given[name];
    }
    // An assistant that runs here goes on with what it read when it started. It is stopped while
    // that is stored, and started again with what is stored then. The window is not closed
    // meanwhile, so that whoever waits for it hears what was stored.
    const running = one.child !== null;
    one.storing = true;
    try {
      await stopAssistant(one);
      const answer = await inTurn(() => runProgram([kind, '--from-stdin'], JSON.stringify(document)));
      if (answer.stored) one.changed = true;
      return answer;
    } finally {
      one.storing = false;
      if (running && !exiting) void startAssistant(one).catch(() => {});
    }
  });
  ipcMain.handle(`columbina-${kind}:open-page`, async (event, address) => {
    checkAssistantSender(event, one);
    // Only a console where what the assistant answers through is made.
    if (typeof address !== 'string' || !one.pages.includes(address)) throw new Error(`That page is not ${one.where}.`);
    await shell.openExternal(address);
  });
  ipcMain.handle(`columbina-${kind}:close`, (event) => {
    checkAssistantSender(event, one);
    one.window.close();
  });
}
/**
 * Stop the sign-in to GitHub, if one runs. What it writes from now on, and its end, are told of
 * to nobody. Its input is closed, which the program takes for a stop, and the program and what
 * it started are ended where it has not ended a few seconds later.
 */
function stopSigningIn() {
  const child = signingIn;
  if (!child) return;
  signingIn = null;
  child.stdin.end();
  const timer = setTimeout(() => killTree(child), SIGN_IN_STOP_MS);
  child.once('close', () => clearTimeout(timer));
}
/** The last line that a program wrote for errors, as far as it has come, without the name that the program puts before it. */
function lastSaid(words) {
  const line = plain(words.rest) || words.lines.at(-1) || '';
  return short(line.replace(/^columbina: /, ''), TAIL_LINE_CHARS);
}
/**
 * Start an assistant, unless the application runs it already. What it says of how it is doing
 * comes to the page a document at a time on `columbina:` and the name of its command, and so does
 * its end: `{ ended: true, code, said }`, with how it ended and the last line that it wrote for
 * errors. What it writes for errors is kept in a log of its own.
 */
async function startAssistant(one) {
  if (askingLoginShell) await askingLoginShell;
  // A stop that is under way ends first: the program allows one assistant of a kind at a time.
  if (one.stopping) await one.stopping;
  // An application that is closing meanwhile starts nothing any more.
  if (one.child || exiting) return;
  const speaks = preferences[`${one.kind}Language`];
  const command = programCommand([one.kind, '--json', ...(speaks ? ['--language', speaks] : [])]);
  if (!command) throw new Error(NO_SETUP);
  mkdirSync(path.dirname(assistantLogPath(one)), { recursive: true });
  const log = createWriteStream(assistantLogPath(one), { flags: 'a' });
  log.on('error', () => {});
  const child = spawn(command.executable, command.args, {
    cwd: app.getPath('home'), env: command.env, windowsHide: true,
    detached: process.platform !== 'win32', stdio: ['pipe', 'pipe', 'pipe'],
  });
  one.child = child;
  one.ended = null;
  const words = listenForErrors(child);
  child.stderr.pipe(log);
  const lines = createInterface({ input: child.stdout, crlfDelay: Infinity });
  lines.on('line', (line) => {
    // Each line is a document of JSON: how the assistant is doing. No line holds a secret.
    if (one.child !== child || line.length > ASSISTANT_LINE_MOST) return;
    let said = null;
    try { said = JSON.parse(line); } catch { return; }
    if (said !== null && typeof said === 'object' && !Array.isArray(said)) tell(`columbina:${one.kind}`, said);
  });
  // An assistant that ended before it read is told of by how it ended.
  child.stdin.on('error', () => {});
  child.on('close', (code, signal) => {
    lines.close();
    log.end();
    if (one.child !== child) return;
    one.child = null;
    const ended = { code: code ?? signal ?? null, said: lastSaid(words) };
    if (ended.code !== 0) one.ended = ended;
    tell(`columbina:${one.kind}`, { ended: true, ...ended });
  });
  let started = false;
  await new Promise((resolve, reject) => {
    child.once('spawn', () => { started = true; resolve(); });
    child.on('error', (error) => {
      // An assistant that started ends by itself, and its end is told of. One that did not never will.
      if (started) return;
      if (one.child === child) one.child = null;
      log.end();
      one.ended = { code: null, said: error.message };
      reject(new Error(`The assistant in ${one.name} could not be started: ${error.message}`));
    });
  });
}
/**
 * Stop an assistant that the application runs, if it runs. Its input is closed, which it takes for
 * a stop, and it and what it started are ended where it has not ended a few seconds later. The
 * promise is kept once it has ended, and broken where it is still there after that.
 */
function stopAssistant(one) {
  if (one.stopping) return one.stopping;
  const child = one.child;
  if (!child) return Promise.resolve();
  one.stopping = new Promise((resolve, reject) => {
    let timer = null;
    const done = () => { clearTimeout(timer); resolve(); };
    child.once('close', done);
    timer = setTimeout(() => {
      killTree(child);
      timer = setTimeout(() => {
        child.removeListener('close', done);
        reject(new Error(`The assistant in ${one.name} did not end.`));
      }, ASSISTANT_KILL_MS);
    }, ASSISTANT_STOP_MS);
    child.stdin.end();
  }).finally(() => { one.stopping = null; });
  return one.stopping;
}
/** Stop every assistant that the application runs. */
const stopAssistants = () => Promise.allSettled(Object.values(assistants).map(stopAssistant));
/** Remember whether an assistant is to run when the application starts. */
function keepAssistantOn(one, on) {
  if (preferences[`${one.kind}On`] === on) return;
  preferences[`${one.kind}On`] = on;
  keepPreferences();
}
/** Remember the language that an assistant says things in by itself, or none. */
function keepAssistantLanguage(one, language) {
  if (preferences[`${one.kind}Language`] === language) return;
  preferences[`${one.kind}Language`] = language;
  keepPreferences();
}
/** Whether the application can start with the system: a package on Windows. */
function loginItemOffered() {
  return app.isPackaged && process.platform === 'win32';
}
/** Whether the application starts with the system, in the tray, where it can. */
function loginItem() {
  if (!loginItemOffered()) return { offered: false, on: false };
  return { offered: true, on: app.getLoginItemSettings({ path: process.execPath, args: [HIDDEN] }).openAtLogin === true };
}
/** The words of the tray: in the language of the page, as it said last, or of the system. */
function trayWords() {
  const system = String(app.getLocale());
  return WORDS[preferences.language ?? (system.startsWith('zh') ? 'zh-CN' : system.startsWith('ja') ? 'ja' : 'en')];
}
/** The menu of the tray, in the words of the moment. */
function trayMenu() {
  const words = trayWords();
  tray.setContextMenu(Menu.buildFromTemplate([
    { label: words.open, click: showWindow },
    { type: 'separator' },
    { label: words.quit, click: quitFromTray },
  ]));
}
/**
 * Put the window away, with an icon in the tray that brings it back. What runs goes on, the
 * assistants in Feishu and in WeCom among it, and what plays in the page, such as the music, is not heard.
 */
function toTray() {
  if (!tray) {
    const image = process.platform === 'win32' ? path.join(__dirname, 'icon.ico')
      : nativeImage.createFromPath(path.resolve(__dirname, '../dist/columbina-icon.png')).resize({ width: 16, height: 16 });
    tray = new Tray(image);
    tray.setToolTip('Columbina');
    tray.on('click', showWindow);
    trayMenu();
  }
  window.hide();
  window.webContents.setAudioMuted(true);
  // The floating scopes, a child of the window, would stay over nothing and measure for no one.
  closeScopesWindow();
}
/** Show the window: from the tray, or where the application was started again while it ran. */
function showWindow() {
  if (!window || window.isDestroyed()) {
    wanted = true;
    return;
  }
  window.webContents.setAudioMuted(false);
  if (window.isMinimized()) window.restore();
  window.show();
  window.focus();
}
/** Quit from the tray: the window closes as it does where nothing keeps it, and the assistant stops. */
function quitFromTray() {
  quitting = true;
  window.close();
}
/** How far one step of zoom goes, as the menu of Electron zooms. */
const ZOOM_STEP = 0.5;
/**
 * What a key did through the menu that the window no longer has, where it is such a key: Ctrl with
 * plus, minus or 0 zooms the page, F11 puts the window in full screen and out of it; and from source,
 * Ctrl+R loads the page again and Ctrl+Shift+I or F12 shows its tools. Null for any other key.
 * F10, which brings a menu to a window of Windows that has one, is no such key: this window has
 * none, and F10 comes to the page with the keys after it, as a real key pressed on Windows showed.
 */
function menuAction(input) {
  if (input?.type !== 'keyDown' || input.alt || input.meta) return null;
  const key = input.key;
  const code = input.code;
  if (input.control && !input.shift && (key === '0' || code === 'Numpad0')) return 'zoom-reset';
  if (input.control && (key === '+' || key === '=' || code === 'NumpadAdd')) return 'zoom-in';
  if (input.control && !input.shift && (key === '-' || code === 'NumpadSubtract')) return 'zoom-out';
  if (!input.control && !input.shift && key === 'F11') return 'full-screen';
  if (app.isPackaged) return null;
  if (input.control && !input.shift && (key === 'r' || key === 'R')) return 'reload';
  if (input.control && input.shift && (key === 'i' || key === 'I')) return 'tools';
  if (!input.control && !input.shift && key === 'F12') return 'tools';
  return null;
}
/**
 * The keys that the page may want for itself: the studio zooms its timeline with Ctrl and plus or
 * minus, and F11 and F12 replace and place on top, as DaVinci Resolve has them. These are left to
 * the page, which asks for what the menu did only where it did not take the key (`pageLeftKey`).
 * Ctrl+R and Ctrl+Shift+I are taken here at once: the page has no use for them, and they must work
 * where the page itself does not.
 */
const PAGE_FIRST = new Set(['zoom-reset', 'zoom-in', 'zoom-out', 'full-screen']);
const pageFirst = (action, input) => PAGE_FIRST.has(action) || (action === 'tools' && input.key === 'F12');
/** Do what the menu did for a key. */
function doMenuAction(action) {
  const page = window.webContents;
  const zoom = (level) => {
    page.setZoomLevel(level);
    // The pixels of the screen that the viewer covers change with the zoom: the page measures it again.
    tell('columbina:engine-event', { kind: 'measure', session: studio?.number ?? null });
  };
  if (action === 'zoom-reset') zoom(0);
  else if (action === 'zoom-in') zoom(page.getZoomLevel() + ZOOM_STEP);
  else if (action === 'zoom-out') zoom(page.getZoomLevel() - ZOOM_STEP);
  else if (action === 'full-screen') window.setFullScreen(!window.isFullScreen());
  else if (action === 'reload') page.reload();
  else if (action === 'tools') page.toggleDevTools();
}
/** A key that comes to the window: whether it was taken here, before the page. */
function pressed(input) {
  const action = menuAction(input);
  if (action === null || pageFirst(action, input)) return false;
  doMenuAction(action);
  return true;
}
/**
 * The page did not take a key that it was left: what the menu did for it is done now. Only such a
 * key is taken, described as `before-input-event` describes it.
 */
function pageLeftKey(asked) {
  const input = {
    type: 'keyDown', key: typeof asked?.key === 'string' ? asked.key : '', code: typeof asked?.code === 'string' ? asked.code : '',
    control: asked?.control === true, shift: asked?.shift === true, alt: asked?.alt === true, meta: asked?.meta === true,
  };
  const action = menuAction(input);
  if (action === null || !pageFirst(action, input)) return false;
  doMenuAction(action);
  return true;
}
/** Stop what runs and quit, without the page: the window has gone, or the system ends the session. */
function endAll() {
  exiting = true;
  stopSigningIn();
  void Promise.allSettled([enqueueService(stopService), stopAssistants(), inStudioTurn(closeStudio)]).then(() => app.quit());
}
function finishWindowClose() {
  clearTimeout(closeTimer);
  if (window && !window.isDestroyed()) window.destroy();
  // An assistant is given its time to stop showing that it writes an answer, and the engine its
  // time to save the project.
  if (anyAssistant() || studio) void Promise.allSettled([stopAssistants(), inStudioTurn(closeStudio)]).then(() => app.quit());
  else app.quit();
}
function requestWindowClose() {
  if (exiting) return;
  exiting = true;
  tell('columbina:shutdown');
  // A responsive renderer finishes immediately through the stop handler. This
  // timer only covers a missing renderer or an unanswered shutdown request.
  closeTimer = setTimeout(() => { void enqueueService(stopService).finally(finishWindowClose); }, 1800);
}
// One application at a time. Another start shows the window of the first, also from the tray, and ends.
const alone = app.requestSingleInstanceLock();
if (alone) app.on('second-instance', showWindow);
else app.quit();
app.whenReady().then(() => {
  if (!alone) return;
  if (!existsSync(indexPath)) throw new Error('Build the desktop client before starting Electron: npm run build');
  installHandlers();
  preferences = readPreferences();
  // The window opens in the theme that was chosen last, so that it does not begin in another one.
  const theme = titleBarThemes[preferences.theme];
  if (process.platform === 'darwin') {
    // On macOS the menu stands at the top of the screen, and the keys that copy and paste, close
    // the window or leave go through it. These are all the menu that the application has there.
    Menu.setApplicationMenu(Menu.buildFromTemplate([{ role: 'appMenu' }, { role: 'fileMenu' }, { role: 'editMenu' }, { role: 'windowMenu' }]));
    if (app.isPackaged) {
      // A package is opened from the Finder, with a short PATH and no proxy. The login shell is asked while the window opens.
      askingLoginShell = askLoginShell().then((found) => { loginShell = found; askingLoginShell = null; });
    } else {
      // A package has its icon in itself. In development the Dock would show that of Electron.
      app.dock?.setIcon(path.resolve(__dirname, '../dist/columbina-icon.png'));
    }
  }
  // Started at login, the application waits in the tray, and its window shows when it is asked for.
  const hidden = process.argv.includes(HIDDEN);
  window = new BrowserWindow({
    width: 1380, height: 920, minWidth: 860, minHeight: 640, backgroundColor: theme.color, title: 'Columbina', show: !hidden,
    icon: path.join(__dirname, 'icon.ico'),
    ...windowBar(theme),
    webPreferences: { preload: path.join(__dirname, 'preload.cjs'), contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true, autoplayPolicy: 'no-user-gesture-required' },
  });
  window.setMenuBarVisibility(false);
  // The engine draws the viewer in a window of its own at the rectangle that the page gives, from
  // the corner of the client area. A menu, which Alt would bring back, would take the top of it.
  // On macOS the menu is the application's, at the top of the screen, and stays.
  if (process.platform !== 'darwin') {
    window.removeMenu();
    // What the menu gave by its keys, given again without it.
    window.webContents.on('before-input-event', (event, input) => { if (pressed(input)) event.preventDefault(); });
  }
  if (process.platform === 'darwin') {
    // A sheet, such as the dialog for a folder or the window for the key, comes down from under the
    // bar that the page draws, and not over it.
    window.setSheetOffset(theme.height);
    // In full screen the buttons of the window are hidden, and the page gives their room back.
    window.on('enter-full-screen', () => tell('columbina:full-screen', true));
    window.on('leave-full-screen', () => tell('columbina:full-screen', false));
  }
  // The production window only loads bundled assets. Vite's development CSP
  // can permit its local HMR socket; the desktop window has no network channel.
  window.webContents.session.webRequest.onBeforeRequest((details, callback) => {
    // User-selected music is stored locally and played through an object URL.
    if (details.url.startsWith('blob:') && details.resourceType === 'media') {
      callback({ cancel: false }); return;
    }
    try {
      const relative = path.relative(path.dirname(indexPath), fileURLToPath(details.url));
      callback({ cancel: relative.startsWith('..') || path.isAbsolute(relative) });
    } catch { callback({ cancel: true }); }
  });
  window.webContents.session.webRequest.onHeadersReceived((details, callback) => callback({
    responseHeaders: {
      ...details.responseHeaders,
      'Content-Security-Policy': ["default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src 'self' blob:; font-src 'self'; connect-src 'none'; object-src 'none'; base-uri 'self'; form-action 'none'; frame-src 'none'"],
    },
  }));
  closeToTheWorld(window);
  window.on('close', (event) => {
    if (exiting) return;
    // While an assistant runs, the window is put away in the tray and the assistant goes on.
    // Quitting from the tray ends them all.
    if (anyAssistant() && !quitting) {
      event.preventDefault();
      toTray();
      return;
    }
    if (service || anyAssistant() || pendingServiceOperations) {
      event.preventDefault();
      requestWindowClose();
    }
  });
  // Windows ends the session: what runs is stopped without waiting for the page.
  window.on('session-end', endAll);
  // The pet goes with the window: the application is not kept open by the pet alone.
  window.on('closed', closePet);
  // The view of the engine went with the window: the engine of the studio is closed with it.
  window.on('closed', () => { void inStudioTurn(closeStudio); });
  if (preferences.petOn) openPet();
  if (hidden && wanted) showWindow();
  else if (hidden) toTray();
  window.webContents.on('render-process-gone', () => { blankView(); dropScopes(); endPreview(); closeScopesWindow(); stopSigningIn(); void enqueueService(stopService); });
  // The view of the engine is hidden while the page is loaded again, until the new page says where the viewer is.
  window.webContents.on('did-start-navigation', (details, _url, inPlace, mainFrame) => {
    const main = details?.isMainFrame ?? mainFrame;
    const same = details?.isSameDocument ?? inPlace;
    if (main === true && same !== true) { blankView(); dropScopes(); endPreview(); closeScopesWindow(); }
  });
  void window.loadFile(indexPath);
  // An assistant runs again where it ran when the application was left.
  for (const one of Object.values(assistants)) {
    if (preferences[`${one.kind}On`]) void startAssistant(one).catch(() => {});
  }
});
app.on('before-quit', (event) => { if ((service || anyAssistant() || pendingServiceOperations) && !exiting) { event.preventDefault(); requestWindowClose(); } });
app.on('window-all-closed', endAll);
// However the application ends, the stdin of its engine is closed, so that the engine saves the
// project and ends rather than be left behind.
app.on('will-quit', () => { stopMigration(studio); studio?.session.abandon(); });
// On macOS a click on the icon in the Dock shows the window that was put away.
app.on('activate', showWindow);
