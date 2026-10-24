/**
 * What the window for the bot of QQ makes of what is typed in it and of what the program
 * answers. Nothing here draws, so all of it can be tried with plain values.
 */
import { blankInside } from '../key/form';

/** What the form holds. The Secret is what is typed in its field, and is never given to the field. */
export interface Form {
  appId: string;
  secret: string;
}

/** The open platform of QQ, where a bot is made. The shell opens this and no other. */
export const CONSOLE = 'https://q.qq.com/';

const fields = (value: unknown): Record<string, unknown> =>
  (value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {});

/**
 * The form that the window opens with, from what `columbina qq --status --json` says: the bot
 * that is set up, or an empty form. With it, whether a bot is set up.
 */
export function formOf(status: unknown): { form: Form; setUp: boolean } {
  const given = fields(status);
  const setUp = given.set_up === true && typeof given.app_id === 'string';
  return { form: { appId: setUp ? given.app_id as string : '', secret: '' }, setUp };
}

/**
 * Whether an AppID is one, as the program checks it: visible characters of ASCII and nothing
 * else, and not too many. The blanks around it are not sent.
 */
export function appIdFits(appId: string): boolean {
  return /^[\x21-\x7e]{1,64}$/.test(appId.trim());
}

/** Whether the form holds what the program needs to check and store a bot. */
export function complete(form: Form): boolean {
  return appIdFits(form.appId) && form.secret.trim() !== '' && !blankInside(form.secret);
}

/** What is handed to the program that checks and stores the bot, as `columbina qq --from-stdin` reads it. */
export function documentOf(form: Form): { app_id: string; secret: string } {
  return { app_id: form.appId.trim(), secret: form.secret.trim() };
}

/** What the program answered: the bot that it stored and where the AppSecret went, or what is wrong. */
export type Stored =
  | { kind: 'stored'; appId: string; secretIn: 'keyring' | 'file' | null }
  | { kind: 'refused'; said: string };

/** Read what `columbina qq --from-stdin` answered. It never holds the AppSecret. */
export function storedOf(answer: unknown): Stored {
  const given = fields(answer);
  const stored = fields(given.stored);
  if (typeof stored.app_id === 'string') {
    const secretIn = given.secret_in === 'keyring' || given.secret_in === 'file' ? given.secret_in : null;
    return { kind: 'stored', appId: stored.app_id, secretIn };
  }
  return { kind: 'refused', said: typeof given.error === 'string' ? given.error : '' };
}
