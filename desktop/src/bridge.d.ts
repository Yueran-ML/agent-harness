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
  /** Start the service for a conversation that belongs to no project, in a folder that the shell names. */
  startChat(): Promise<{ mode: 'mock' | 'service'; cwd: string }>;
  send(line: string): void;
  onLine(listener: (line: string) => void): () => void;
  onClose(listener: (reason: string) => void): () => void;
  onShutdown(listener: () => void): () => void;
  /** Hear when the window goes into full screen, and out of it. Told of on macOS only. */
  onFullScreen(listener: (full: boolean) => void): () => void;
  stop(): Promise<void>;
  setTheme(theme: 'moonveil' | 'moonwhite'): Promise<void>;
  /** Tell the shell the language of the page, in which the menu of the tray speaks. */
  setLanguage(language: string): Promise<void>;
  /** Whether the application can start with the system, in the tray, and whether it does. */
  loginItem(): Promise<unknown>;
  /** Have the application start with the system, in the tray, or not. It answers as `loginItem`. */
  setLoginItem(on: boolean): Promise<unknown>;
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
   * What `columbina balance --json` says of a service that the window was told of: how much money
   * the account of its key has left, that the service does not say, or why it could not be asked.
   * It holds no key.
   */
  balance(service: string): Promise<unknown>;
  /**
   * Have the model reason in a language from the next turn on, in every conversation, or in
   * whatever language suits it, for `none`. It fails where the choice cannot be kept.
   */
  setReasoningLanguage(language: 'zh' | 'en' | 'ja' | 'none'): Promise<unknown>;
  /**
   * Keep a model as the one of a service that `services` named, for the next start, with
   * `columbina setup --from-stdin`. The key of the service stays as it is.
   */
  chooseModel(asked: { service: string; model: string }): Promise<unknown>;
  /**
   * Keep how hard the model of a service that `services` named is to think, for the next start,
   * with `columbina setup --from-stdin`. All else that was chosen stays, and so does the service
   * that is in use. Ultra holds for one run, and is not kept.
   */
  chooseEffort(asked: { service: string; effort: string }): Promise<unknown>;
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
  /** The version of the application that runs, and whether it looks for newer ones: a package does, and a checkout does not. */
  about(): Promise<unknown>;
  /**
   * Ask which release of the application is the newest: the version that comes after the one that
   * runs, with the page of its release, or none, or why that is not known. The shell takes an
   * answer of the last hour as it is, unless it is asked now.
   */
  updates(asked: { now: boolean }): Promise<unknown>;
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
  /**
   * What `columbina feishu --status --json` says: which app of Feishu is set up and how its
   * assistant is doing, without the App Secret. With `own`, whether this application runs the
   * assistant; with `on`, whether it starts it when it starts; with `ended`, how the last one that
   * it ran ended badly, or why it did not start. Null where the application does not run the service.
   */
  feishuStatus(): Promise<unknown>;
  /**
   * Start the assistant in Feishu, which then runs whenever the application runs, until it is
   * stopped. What it says of how it is doing comes as documents to `onFeishu`, and so does its end.
   * What it says by itself, such as its help, it says in the language of the page where it can.
   */
  feishuStart(asked: { language: string }): Promise<void>;
  /** Stop the assistant in Feishu that this application runs. It no longer starts with the application. */
  feishuStop(): Promise<void>;
  /** Stop the assistant in Feishu, and delete its app, its App Secret and whom it answered. */
  feishuForget(): Promise<unknown>;
  /** Show the log of the assistant in Feishu in its folder. */
  feishuLog(): Promise<void>;
  /**
   * Say whether the agent of the assistant in Feishu may change the files of its folder. One that
   * runs is stopped first and started again after. The answer is what the program kept, or why not.
   */
  feishuWritable(writable: boolean): Promise<unknown>;
  /** Open the folder of the assistant in Feishu, as the program names it. */
  openFeishuFolder(): Promise<void>;
  /**
   * Open the window in which the app of Feishu and its App Secret are given, and wait until it is
   * closed. This window is told whether an app was stored there, and nothing of it.
   */
  setUpFeishu(asked: { language: string }): Promise<{ changed: boolean }>;
  /**
   * Hear how the assistant in Feishu that the application runs is doing: its state, whether it is
   * paired, the code of pairing and what went wrong. Its end comes as `{ ended: true, code, said }`.
   */
  onFeishu(listener: (said: Record<string, unknown>) => void): () => void;
  /**
   * Choose the model that the assistant in Feishu asks and how hard it thinks, or follow what holds
   * for all with null. One that runs starts again for it. What the program answered comes back.
   */
  feishuThinking(asked: { model: string | null; effort: 'default' | 'off' | 'low' | 'high' | 'max' | null }): Promise<unknown>;
  /**
   * What `columbina wecom --status --json` says: which bot of WeCom is set up and how its assistant
   * is doing, without the Secret, with what the shell adds as to `feishuStatus`.
   */
  wecomStatus(): Promise<unknown>;
  /** Start the assistant in WeCom, as `feishuStart` starts the one in Feishu. Its lines come to `onWecom`. */
  wecomStart(asked: { language: string }): Promise<void>;
  /** Stop the assistant in WeCom that this application runs. It no longer starts with the application. */
  wecomStop(): Promise<void>;
  /** Stop the assistant in WeCom, and delete its bot, its Secret and whom it answered. */
  wecomForget(): Promise<unknown>;
  /** Show the log of the assistant in WeCom in its folder. */
  wecomLog(): Promise<void>;
  /** Say whether the agent of the assistant in WeCom may change the files of its folder, as `feishuWritable` does. */
  wecomWritable(writable: boolean): Promise<unknown>;
  /** Open the folder of the assistant in WeCom, as the program names it. */
  openWecomFolder(): Promise<void>;
  /**
   * Open the window in which the bot of WeCom and its Secret are given, and wait until it is
   * closed. This window is told whether a bot was stored there, and nothing of it.
   */
  setUpWecom(asked: { language: string }): Promise<{ changed: boolean }>;
  /** Hear how the assistant in WeCom that the application runs is doing, as `onFeishu` does of the one in Feishu. */
  onWecom(listener: (said: Record<string, unknown>) => void): () => void;
  /**
   * Choose the model that the assistant in WeCom asks and how hard it thinks, or follow what holds
   * for all with null. One that runs starts again for it. What the program answered comes back.
   */
  wecomThinking(asked: { model: string | null; effort: 'default' | 'off' | 'low' | 'high' | 'max' | null }): Promise<unknown>;
  /**
   * What `columbina qq --status --json` says: which bot of QQ is set up and how its assistant is
   * doing, without the AppSecret, with what the shell adds as to `feishuStatus`.
   */
  qqStatus(): Promise<unknown>;
  /** Start the assistant in QQ, as `feishuStart` starts the one in Feishu. Its lines come to `onQq`. */
  qqStart(asked: { language: string }): Promise<void>;
  /** Stop the assistant in QQ that this application runs. It no longer starts with the application. */
  qqStop(): Promise<void>;
  /** Stop the assistant in QQ, and delete its bot, its AppSecret and whom it answered. */
  qqForget(): Promise<unknown>;
  /** Show the log of the assistant in QQ in its folder. */
  qqLog(): Promise<void>;
  /** Open the folder of the assistant in QQ, as the program names it. */
  openQqFolder(): Promise<void>;
  /**
   * Open the window in which the bot of QQ and its AppSecret are given, and wait until it is
   * closed. This window is told whether a bot was stored there, and nothing of it.
   */
  setUpQq(asked: { language: string }): Promise<{ changed: boolean }>;
  /** Hear how the assistant in QQ that the application runs is doing, as `onFeishu` does of the one in Feishu. */
  onQq(listener: (said: Record<string, unknown>) => void): () => void;
  /**
   * Choose the model that the assistant in QQ asks and how hard it thinks, or follow what holds
   * for all with null. One that runs starts again for it. What the program answered comes back.
   */
  qqThinking(asked: { model: string | null; effort: 'default' | 'off' | 'low' | 'high' | 'max' | null }): Promise<unknown>;
}

declare global {
  interface Window { columbina?: ColumbinaBridge }
}
