/**
 * What the page makes of the assistant in Feishu: which app is set up and how its assistant is
 * doing, as `columbina feishu --status --json` says with what the shell adds, and how the
 * assistant that the application runs tells of itself line by line. Nothing here draws, so all
 * of it can be tried with plain values.
 */

/** How an assistant that the application ran ended badly, or why it did not start. */
export interface Ended {
  /** How the program ended: its code, the name of the signal that ended it, or null where it did not start. */
  code: number | string | null;
  /** The last line that it wrote for errors. */
  said: string;
}

/** The two platforms: Feishu in China, and Lark outside it. */
export type Platform = 'feishu' | 'lark';

/** How the assistant is doing, while it runs. */
export type State = 'connecting' | 'connected' | 'stopping' | 'failed';

/** The app that is set up, and its assistant. */
export interface App {
  platform: Platform;
  appId: string;
  /** The developer console of the platform, where the app is made and changed. */
  console: string;
  /** Whether somebody paired with the assistant, whom it answers. */
  paired: boolean;
  /** Whether an assistant runs on this computer, here or elsewhere. */
  running: boolean;
  /** Whether this application runs it. */
  own: boolean;
  /** How it is doing, where it runs and has said so, or how it ended where it failed. */
  state: State | null;
  /** The code of pairing, while the assistant waits for somebody to send it. */
  code: string | null;
  /** What went wrong last, as the program said it. */
  error: string | null;
  /** How the last assistant that the application ran ended badly. */
  ended: Ended | null;
}

/** The assistant in Feishu, as far as the page knows. */
export type Assistant =
  /** Not asked yet, or the answer has not come. */
  | { kind: 'unknown' }
  /** There is no service program to ask, as in a replay. */
  | { kind: 'unavailable' }
  /** What is kept of the app could not be read, or the program could not be run. */
  | { kind: 'failed'; said: string }
  /** No app is set up. */
  | { kind: 'none'; ended: Ended | null }
  | ({ kind: 'app' } & App);

const text = (value: unknown): string => (typeof value === 'string' ? value : '');
const fields = (value: unknown): Record<string, unknown> =>
  (value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {});
/** The consoles that the program names. Nothing else is offered to be opened. */
const CONSOLES: Record<Platform, string> = {
  feishu: 'https://open.feishu.cn/app',
  lark: 'https://open.larksuite.com/app',
};
const platformOf = (value: unknown): Platform => (value === 'lark' ? 'lark' : 'feishu');
const codeOf = (value: unknown): string | null => (/^\d{6}$/.test(text(value)) ? text(value) : null);

/** How an assistant ended, from what the shell says of it, or null where it says nothing of an end. */
export function endedOf(value: unknown): Ended | null {
  const given = fields(value);
  if (!('code' in given) && !('said' in given)) return null;
  const { code } = given;
  return {
    code: typeof code === 'number' || typeof code === 'string' ? code : null,
    said: text(given.said),
  };
}

/** How the assistant is doing, from what the program says, where it runs. */
function stateOf(value: unknown, running: boolean): State | null {
  if (value === 'failed') return 'failed';
  if (!running) return null;
  if (value === 'connecting' || value === 'connected') return value;
  return value === 'stopped' ? 'stopping' : null;
}

/**
 * Read what `columbina feishu --status --json` answered, with what the shell adds: whether the
 * application runs the assistant and how the last one that it ran ended badly. Null is the
 * answer where there is no service program.
 */
export function assistantOf(answer: unknown): Assistant {
  if (answer === null || typeof answer !== 'object') return { kind: 'unavailable' };
  const given = fields(answer);
  // The program says why it could not answer in a document of its own. The status of an app
  // says what went wrong with the assistant under the same name.
  if (!('set_up' in given)) return { kind: 'failed', said: text(given.error) };
  const ended = endedOf(given.ended);
  if (given.set_up !== true) return { kind: 'none', ended };
  const platform = platformOf(given.domain);
  const own = given.own === true;
  const running = given.running === true || own;
  return {
    kind: 'app',
    platform,
    appId: text(given.app_id),
    console: CONSOLES[platform],
    paired: given.paired === true,
    running,
    own,
    state: stateOf(given.state, running),
    code: running ? codeOf(given.code) : null,
    error: text(given.error) || null,
    ended: own ? null : ended,
  };
}

/**
 * The assistant after something that the assistant which the application runs said: how it is
 * doing, or that it ended. Null where the page knows of no app to which that belongs, so that
 * it asks again.
 */
export function assistantAfter(current: Assistant, told: Record<string, unknown>): Assistant | null {
  if (current.kind !== 'app') return null;
  // The assistant of another app than the page knows of, as after the app was changed.
  const { domain, app_id: appId } = told;
  if ((typeof appId === 'string' && appId !== current.appId)
    || ((domain === 'feishu' || domain === 'lark') && domain !== current.platform)) return null;
  if (told.ended === true) {
    const ended = endedOf(told);
    const failed = current.state === 'failed';
    return {
      ...current,
      running: false,
      own: false,
      state: failed ? 'failed' : null,
      code: null,
      // An assistant that was stopped leaves nothing to say. One that failed said why before it ended.
      error: failed ? current.error : null,
      ended: ended !== null && ended.code !== 0 ? ended : null,
    };
  }
  return {
    ...current,
    running: true,
    own: true,
    state: stateOf(told.state, true),
    paired: told.paired === true,
    code: codeOf(told.code),
    error: text(told.error) || null,
    ended: null,
  };
}

/** What the panel says of an assistant that runs, as the name of the words that say it. */
export type Doing = 'connecting' | 'reconnecting' | 'pair' | 'noCode' | 'connected' | 'stopping' | 'elsewhere';

/** What an assistant that runs is doing, as the panel says it. */
export function doingOf(app: App): Doing {
  if (!app.own) return 'elsewhere';
  if (app.state === 'stopping') return 'stopping';
  if (app.state !== 'connected') return app.error !== null ? 'reconnecting' : 'connecting';
  if (app.paired) return 'connected';
  return app.code !== null ? 'pair' : 'noCode';
}

