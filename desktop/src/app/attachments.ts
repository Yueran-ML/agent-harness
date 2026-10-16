/**
 * The files that go with a message, as the page holds them: what the shell says of them, how a
 * size is written, which of them are pictures, and the words that the message ends with. Every
 * task keeps the files of its message, as it keeps its draft. Nothing here draws.
 */
import { nameOf, type Place } from './drafts';

/** A file that the shell holds for a message, as the page knows it: by an id, and never where it lies. */
export interface Attachment {
  id: string;
  name: string;
  size: number;
}

/** Why the shell took no file: it was gone, a folder, too large, or one too many. */
export const refusals = ['gone', 'notAFile', 'tooBig', 'tooMany'] as const;
export type Refusal = typeof refusals[number];

/** A file that was not taken, by its name, and why. */
export interface Refused {
  name: string;
  why: Refusal;
}

/** How many files go with one message. */
export const ATTACHMENTS_MOST = 10;

const text = (value: unknown) => (typeof value === 'string' ? value : '');

/** Read what the shell answers when files are chosen or dropped. What it does not say is nothing. */
export function attachedOf(answer: unknown): { attached: Attachment[]; refused: Refused[] } {
  const given = (answer !== null && typeof answer === 'object' ? answer : {}) as { attached?: unknown; refused?: unknown };
  const attached: Attachment[] = [];
  for (const each of Array.isArray(given.attached) ? given.attached as unknown[] : []) {
    const file = (each !== null && typeof each === 'object' ? each : {}) as Record<string, unknown>;
    const size = typeof file.size === 'number' && Number.isFinite(file.size) ? file.size : -1;
    if (text(file.id) !== '' && text(file.name) !== '' && size >= 0) attached.push({ id: text(file.id), name: text(file.name), size });
  }
  const refused: Refused[] = [];
  for (const each of Array.isArray(given.refused) ? given.refused as unknown[] : []) {
    const file = (each !== null && typeof each === 'object' ? each : {}) as Record<string, unknown>;
    const why = refusals.find(kind => kind === file.why);
    if (text(file.name) !== '' && why !== undefined) refused.push({ name: text(file.name), why });
  }
  return { attached, refused };
}

/** Read where the shell says the files of a message are now, from the folder of the project. Null for no such answer. */
export function keptOf(answer: unknown, count: number): string[] | null {
  const given = (answer !== null && typeof answer === 'object' ? answer : {}) as { kept?: unknown };
  if (!Array.isArray(given.kept) || given.kept.length !== count) return null;
  const kept = given.kept as unknown[];
  // A path is a line of the message: what cannot be seen, such as the end of a line, is in none.
  return kept.every(each => text(each) !== '' && !/[\u0000-\u001f\u007f]/.test(text(each))) ? kept as string[] : null;
}

/**
 * Add the files that came to those of a message, as many as go with it. Returns the files of the
 * message, and those that came and were one too many.
 */
export function added(files: readonly Attachment[], coming: readonly Attachment[]): { files: Attachment[]; over: Attachment[] } {
  const room = Math.max(0, ATTACHMENTS_MOST - files.length);
  return { files: [...files, ...coming.slice(0, room)], over: coming.slice(room) };
}

/** The size of a file, as a person reads it: 900 B, 12 KB, 3.4 MB, 25 MB. */
export function sizeOf(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  const megabytes = bytes / (1024 * 1024);
  return megabytes < 10 ? `${megabytes.toFixed(1)} MB` : `${Math.round(megabytes)} MB`;
}

const PICTURES = /\.(png|jpe?g|gif|webp|bmp|heic|heif|tiff?|avif|ico)$/i;

/** Whether a file is a picture, by the ending of its name. Of a picture the model has the file, and not what it shows. */
export function isPicture(name: string): boolean {
  return PICTURES.test(name);
}

/**
 * The message with the files that go with it: what was written, then the heading and a list of
 * where each file is, from the folder of the project, a line for each. A message of files alone
 * is the list. The user sees the message as it is written, so the list has no marks of Markdown
 * beyond its dashes.
 */
export function withAttachments(written: string, heading: string, kept: readonly string[]): string {
  const list = [heading, ...kept.map(where => `- ${where}`)].join('\n');
  const said = written.trim();
  return said === '' ? list : `${said}\n\n${list}`;
}

/** The files of the messages that are being written. Every task keeps its own, as it keeps its draft. */
export class Attachments {
  private readonly kept = new Map<string, Attachment[]>();

  of(place: Place): Attachment[] {
    return this.kept.get(nameOf(place)) ?? [];
  }

  /** Keep the files of the message of a task. No files is nothing to keep. */
  keep(place: Place, files: readonly Attachment[]): void {
    if (files.length === 0) this.kept.delete(nameOf(place));
    else this.kept.set(nameOf(place), [...files]);
  }
}
