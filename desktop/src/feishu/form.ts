/**
 * What the window for the app of Feishu makes of what is typed in it and of what the program
 * answers. Nothing here draws, so all of it can be tried with plain values.
 */

/** The two platforms: Feishu in China, and Lark outside it. */
export type Platform = 'feishu' | 'lark';

/** What the form holds. The App Secret is what is typed in its field, and is never given to the field. */
export interface Form {
  platform: Platform;
  appId: string;
  secret: string;
}

/** The developer consoles of the two platforms, where an app is made. The shell opens these and no other. */
export const CONSOLES: Record<Platform, string> = {
  feishu: 'https://open.feishu.cn/app',
  lark: 'https://open.larksuite.com/app',
};

/** The other platform: where an app is that one of the two does not know. */
export const OTHER: Record<Platform, Platform> = { feishu: 'lark', lark: 'feishu' };

/**
 * The permissions that the app is to be given: those for messages, which the assistant needs,
 * and those for reading documents, which it may do without.
 */
export const SCOPES = {
  messages: ['im:message.p2p_msg:readonly', 'im:message:send_as_bot', 'im:message.reactions:write_only', 'im:message:readonly'],
  documents: ['docx:document:readonly', 'wiki:wiki:readonly', 'sheets:spreadsheet:readonly'],
};

const fields = (value: unknown): Record<string, unknown> =>
  (value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {});

/**
 * The form that the window opens with, from what `columbina feishu --status --json` says: the app
 * that is set up, or an empty form on Feishu. With it, whether an app is set up.
 */
export function formOf(status: unknown): { form: Form; setUp: boolean } {
  const given = fields(status);
  const setUp = given.set_up === true && typeof given.app_id === 'string';
  return {
    form: {
      platform: given.domain === 'lark' ? 'lark' : 'feishu',
      appId: setUp ? given.app_id as string : '',
      secret: '',
    },
    setUp,
  };
}

/** Whether an App ID is one, as the program checks it. The blanks around it are not sent. */
export function appIdFits(appId: string): boolean {
  return /^cli_[A-Za-z0-9]{4,60}$/.test(appId.trim());
}

/**
 * Whether an App Secret that was typed or pasted has a blank inside it. None has one, and one
 * that is copied from a page may bring a line of the page along.
 */
export function blankInside(secret: string): boolean {
  return /\s/.test(secret.trim());
}

/** Whether the form holds what the program needs to check and store an app. */
export function complete(form: Form): boolean {
  return appIdFits(form.appId) && form.secret.trim() !== '' && !blankInside(form.secret);
}

/** What is handed to the program that checks and stores the app, as `columbina feishu --from-stdin` reads it. */
export function documentOf(form: Form): { domain: Platform; app_id: string; app_secret: string } {
  return { domain: form.platform, app_id: form.appId.trim(), app_secret: form.secret.trim() };
}

/**
 * What the program answered: the app that it stored and where the App Secret went, or what is
 * wrong, and whether that is that the app is one of the other platform.
 */
export type Stored =
  | { kind: 'stored'; appId: string; secretIn: 'keyring' | 'file' | null }
  | { kind: 'refused'; said: string; elsewhere: boolean };

/** Read what `columbina feishu --from-stdin` answered. It never holds the App Secret. */
export function storedOf(answer: unknown): Stored {
  const given = fields(answer);
  const stored = fields(given.stored);
  if (typeof stored.app_id === 'string') {
    const secretIn = given.secret_in === 'keyring' || given.secret_in === 'file' ? given.secret_in : null;
    return { kind: 'stored', appId: stored.app_id, secretIn };
  }
  return {
    kind: 'refused',
    said: typeof given.error === 'string' ? given.error : '',
    elsewhere: given.code === 'WRONG_PLATFORM',
  };
}

/** The machine that a page is on, as the window shows it beside the button that opens the page. */
export function hostOf(address: string): string {
  try {
    return new URL(address).host;
  } catch {
    return address;
  }
}
