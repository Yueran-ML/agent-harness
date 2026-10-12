export interface ColumbinaBridge {
  selectFolder(): Promise<string | null>;
  getLocalMusic(): Promise<{ name: string; bytes: ArrayBuffer; mimeType: string } | null>;
  start(options: { cwd: string }): Promise<{ mode: 'mock' | 'service'; cwd: string }>;
  send(line: string): void;
  onLine(listener: (line: string) => void): () => void;
  onClose(listener: (reason: string) => void): () => void;
  onShutdown(listener: () => void): () => void;
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
  openExternal(url: string): Promise<void>;
}

declare global {
  interface Window { columbina?: ColumbinaBridge }
}
