/** What the window for the key can ask of the shell. It can ask nothing else of the system. */
export interface KeyBridge {
  /** What is set up, and what the window was opened for. */
  context(): Promise<{ service: string | null; language: string; theme: string; list: unknown }>;
  /**
   * Hand a service, a model and where wanted a key to the program that stores them.
   * The answer says what was stored, or what is wrong. It never holds the key.
   */
  store(document: Record<string, string | number>): Promise<unknown>;
  /**
   * Ask a service for its models, with the key that is given, or with the one that a session
   * would send. Nothing is stored. The answer lists the models, or says what kind of failure
   * it was. It never holds the key.
   */
  check(document: Record<string, string>): Promise<unknown>;
  /** Delete what is stored for a service that the window was told of: its key, and that it was chosen. */
  forget(name: string): Promise<unknown>;
  /** Open the page on which the key of a service is made, as the list of services names it. */
  openPage(address: string): Promise<void>;
  close(): Promise<void>;
}

declare global {
  interface Window { columbinaKey?: KeyBridge }
}
