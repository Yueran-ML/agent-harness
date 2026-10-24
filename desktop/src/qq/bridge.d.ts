/** What the window for the bot of QQ can ask of the shell. It can ask nothing else of the system. */
export interface QqBridge {
  /** The language and theme that the window opens in, and what `columbina qq --status --json` says. */
  context(): Promise<{ language: string; theme: string; status: unknown }>;
  /**
   * Hand the AppID and the AppSecret to the program, which checks them with QQ and stores them.
   * The answer says what was stored, or what is wrong. It never holds the AppSecret.
   */
  store(document: { app_id: string; secret: string }): Promise<unknown>;
  /** Open the open platform of QQ. */
  openPage(address: string): Promise<void>;
  close(): Promise<void>;
}

declare global {
  interface Window { columbinaQq?: QqBridge }
}
