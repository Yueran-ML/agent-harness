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
  openExternal(url: string): Promise<void>;
}

declare global {
  interface Window { columbina?: ColumbinaBridge }
}
