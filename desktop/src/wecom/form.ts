/**
 * What the window for the bot of WeCom makes of what is typed in it and of what the program
 * answers. Nothing here draws, so all of it can be tried with plain values.
 */
import { blankInside } from '../key/form';

/** What the form holds. The Secret is what is typed in its field, and is never given to the field. */
export interface Form {
  botId: string;
  secret: string;
}

/** The admin console of WeCom, where a bot is made. The shell opens this and no other. */
export const CONSOLE = 'https://work.weixin.qq.com/wework_admin/frame';

const fields = (value: unknown): Record<string, unknown> =>
  (value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {});

/**
 * The form that the window opens with, from what `columbina wecom --status --json` says: the bot
 * that is set up, or an empty form. With it, whether a bot is set up.
 */
export function formOf(status: unknown): { form: Form; setUp: boolean } {
  const given = fields(status);
  const setUp = given.set_up === true && typeof given.bot_id === 'string';
  return { form: { botId: setUp ? given.bot_id as string : '', secret: '' }, setUp };
}

/**
 * Whether a Bot ID is one, as the program checks it: visible characters of ASCII and nothing
 * else, and not too many. The blanks around it are not sent.
 */
export function botIdFits(botId: string): boolean {
  return /^[\x21-\x7e]{1,128}$/.test(botId.trim());
}

/** Whether the form holds what the program needs to check and store a bot. */
export function complete(form: Form): boolean {
  return botIdFits(form.botId) && form.secret.trim() !== '' && !blankInside(form.secret);
}

/** What is handed to the program that checks and stores the bot, as `columbina wecom --from-stdin` reads it. */
export function documentOf(form: Form): { bot_id: string; secret: string } {
  return { bot_id: form.botId.trim(), secret: form.secret.trim() };
}

/** What the program answered: the bot that it stored and where the Secret went, or what is wrong. */
export type Stored =
  | { kind: 'stored'; botId: string; secretIn: 'keyring' | 'file' | null }
  | { kind: 'refused'; said: string };

/** Read what `columbina wecom --from-stdin` answered. It never holds the Secret. */
export function storedOf(answer: unknown): Stored {
  const given = fields(answer);
  const stored = fields(given.stored);
  if (typeof stored.bot_id === 'string') {
    const secretIn = given.secret_in === 'keyring' || given.secret_in === 'file' ? given.secret_in : null;
    return { kind: 'stored', botId: stored.bot_id, secretIn };
  }
  return { kind: 'refused', said: typeof given.error === 'string' ? given.error : '' };
}
