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
   * Open the window in which a service is chosen and its key is given, and wait until it is
   * closed. This window is told whether something was stored or deleted there, and nothing of what.
   */
  setUpService(asked: { service: string | null; language: string }): Promise<{ changed: boolean }>;
  openExternal(url: string): Promise<void>;
}

declare global {
  interface Window { columbina?: ColumbinaBridge }
}
