/** What the window for the app of Feishu can ask of the shell. It can ask nothing else of the system. */
export interface FeishuBridge {
  /** The language and theme that the window opens in, and what `columbina feishu --status --json` says. */
  context(): Promise<{ language: string; theme: string; status: unknown }>;
  /**
   * Hand the platform, the App ID and the App Secret to the program, which checks them with
   * Feishu and stores them. The answer says what was stored, or what is wrong. It never holds
   * the App Secret.
   */
  store(document: { domain: string; app_id: string; app_secret: string }): Promise<unknown>;
  /** Open the developer console of one of the two platforms. */
  openPage(address: string): Promise<void>;
  close(): Promise<void>;
}

declare global {
  interface Window { columbinaFeishu?: FeishuBridge }
}
