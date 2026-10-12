/** What the window for the key can ask of the shell. It can ask nothing else of the system. */
export interface KeyBridge {
  /** What is set up, and what the window was opened for. */
  context(): Promise<{ service: string | null; language: string; theme: string; list: unknown }>;
  /**
   * Hand a service, a model and where wanted a key to the program that stores them.
   * The answer says what was stored, or what is wrong. It never holds the key.
   */
  store(document: Record<string, string | number>): Promise<unknown>;
  close(): Promise<void>;
}

declare global {
  interface Window { columbinaKey?: KeyBridge }
}
