/** What the window for the bot of WeCom can ask of the shell. It can ask nothing else of the system. */
export interface WecomBridge {
  /** The language and theme that the window opens in, and what `columbina wecom --status --json` says. */
  context(): Promise<{ language: string; theme: string; status: unknown }>;
  /**
   * Hand the Bot ID and the Secret to the program, which checks them with WeCom and stores them.
   * The answer says what was stored, or what is wrong. It never holds the Secret.
   */
  store(document: { bot_id: string; secret: string }): Promise<unknown>;
  /** Open the admin console of WeCom. */
  openPage(address: string): Promise<void>;
  close(): Promise<void>;
}

declare global {
  interface Window { columbinaWecom?: WecomBridge }
}
