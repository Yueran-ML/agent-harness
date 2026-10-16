export interface ColumbinaBridge {
  /** The system that the application runs on, as Node names it: `win32`, `darwin` or `linux`. */
  platform: string;
  /**
   * Let the user choose a folder in the dialog of the system, which the shell titles in the
   * language that is named. Null if none was chosen.
   */
  selectFolder(asked: { language: string }): Promise<string | null>;
  getLocalMusic(): Promise<{ name: string; bytes: ArrayBuffer; mimeType: string } | null>;
  start(options: { cwd: string }): Promise<{ mode: 'mock' | 'service'; cwd: string }>;
  send(line: string): void;
  onLine(listener: (line: string) => void): () => void;
  onClose(listener: (reason: string) => void): () => void;
  onShutdown(listener: () => void): () => void;
  /** Hear when the window goes into full screen, and out of it. Told of on macOS only. */
  onFullScreen(listener: (full: boolean) => void): () => void;
  stop(): Promise<void>;
  setTheme(theme: 'moonveil' | 'moonwhite'): Promise<void>;
  /** The last lines that the service wrote for errors. They are still there after it has ended. */
  serviceTail(): Promise<string[]>;
  /** Show the log of the service in its folder. */
  showLog(): Promise<void>;
  /** Tell the user of something while the window is in the background. False if the window is looked at. */
  notify(message: { title: string; body: string }): Promise<boolean>;
  /** The folders that were chosen at earlier times and are still there, the last one first. */
  recentProjects(): Promise<{ path: string; name: string }[]>;
  /** Choose one of the recent projects again. The answer is the folder to start in. */
  openRecent(folder: string): Promise<string>;
  /**
   * What `columbina setup --list` says: the services, and where the key of each comes from.
   * It holds no key. Null where the application does not run the service itself.
   */
  services(): Promise<unknown>;
  /**
   * What `columbina setup --check` answers for one of the services that `services` named: the
   * models that it lists, asked for with the key that a session would send, or why it listed
   * none. No key comes this way.
   */
  listModels(service: string): Promise<unknown>;
  /**
   * Keep a model as the one of a service that `services` named, for the next start, with
   * `columbina setup --from-stdin`. The key of the service stays as it is.
   */
  chooseModel(asked: { service: string; model: string }): Promise<unknown>;
  /**
   * Let the user choose files to go with a message, in the dialog of the system, which the shell
   * titles in the language that is named. The answer names the files that the shell holds, by id,
   * name and size, and the others with why not. Null if none was chosen.
   */
  chooseAttachments(asked: { language: string }): Promise<unknown>;
  /** Hold files that were dropped on the window, as `chooseAttachments` does those that are chosen. */
  attachDropped(files: File[]): Promise<unknown>;
  /** Let go of files that are held and are not to go with a message after all. */
  forgetAttachments(ids: string[]): Promise<void>;
  /**
   * Copy files that are held into the project, beside the other files that went with messages.
   * The answer names where each is now, from the folder of the project.
   */
  keepAttachments(ids: string[]): Promise<unknown>;
  /**
   * Open the window in which a service is chosen and its key is given, and wait until it is
   * closed. This window is told whether something was stored or deleted there, and nothing of what.
   */
  setUpService(asked: { service: string | null; language: string }): Promise<{ changed: boolean }>;
  openExternal(url: string): Promise<void>;
  /**
   * What `columbina login --status --json` says: who is signed in to GitHub, with the picture
   * as an address of data, and what Columbina may reach; with `skipped`, whether the user said
   * on the first page that GitHub can wait. Null where the application does not run the service.
   */
  githubStatus(): Promise<unknown>;
  /**
   * Begin a sign-in to GitHub, in the place of one that runs. What it says comes as documents to
   * `onGitHub`: the code and the page to enter it on, then how it ended, then `{ ended: true }`.
   */
  githubSignIn(): Promise<void>;
  /** Stop the sign-in that runs, if one does. Nothing more is told of it. */
  githubCancel(): Promise<void>;
  /** What `columbina logout github --json` says: whether anybody was signed in, and where the right is taken back. */
  githubSignOut(): Promise<unknown>;
  /** Say whether GitHub is to wait, so that the first page offers it no more, or again. */
  skipGitHub(skipped: boolean): Promise<void>;
  onGitHub(listener: (said: Record<string, unknown>) => void): () => void;
}

declare global {
  interface Window { columbina?: ColumbinaBridge }
}
