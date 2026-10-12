/**
 * What the interface works out before it says something: the name of a
 * folder, how long ago, what a call acts on. Nothing here draws anything, so
 * all of it can be tried with plain values.
 */
import type { Messages } from '../messages/en';
import type { ToolItem } from '../state';
import { promptOf, type Turn } from '../turns';
import type { PermissionPreset } from '../uiActions';

/** The last part of a path: the name of the folder. */
export function folderName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}

/** How long ago a moment was. */
export function agoOf(words: Messages, then: number, now: number): string {
  const minutes = Math.floor((now - then) / 60_000);
  // A moment that lies ahead, as the clock of another program may have it, is now too.
  if (minutes < 1) return words.label.ago.now;
  if (minutes < 60) return words.label.ago.minutes(minutes);
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return words.label.ago.hours(hours);
  return words.label.ago.days(Math.floor(hours / 24));
}

/** What a task is called in a list: the first line of what it began with. */
export function taskTitle(words: Messages, firstInput: string | null): string {
  return firstInput?.trim().split('\n')[0].trim() || words.label.newTask;
}

/** Several texts as one, each on lines of its own. A text with nothing in it is left out. */
export function joined(texts: string[]): string {
  return texts.filter(text => text.trim() !== '').join('\n');
}

/**
 * What the agent is given when the user asks it to try a turn again.
 *
 * A turn that had done nothing yet is given its task again. One that had
 * begun is told to go on, because its task was read and part of it is done.
 */
export function retryText(turn: Turn, goOn: string): string {
  const prompt = promptOf(turn);
  const begun = turn.steps.some(item => item.type === 'tool' || item.type === 'assistant');
  return begun || prompt === null ? goOn : prompt.text;
}

/**
 * Where an arrow key leads among things that stand in a row, such as tabs: to
 * the next one or the one before, around the ends, and Home and End to the
 * first and the last. Null for a key that leads nowhere.
 */
export function stepped<T>(row: readonly T[], from: T, key: string): T | null {
  const at = row.indexOf(from);
  if (at < 0 || row.length === 0) return null;
  switch (key) {
    case 'ArrowRight':
    case 'ArrowDown':
      return row[(at + 1) % row.length];
    case 'ArrowLeft':
    case 'ArrowUp':
      return row[(at - 1 + row.length) % row.length];
    case 'Home':
      return row[0];
    case 'End':
      return row[row.length - 1];
    default:
      return null;
  }
}

/**
 * A count of tokens, as short as a language writes it: `2.7K` in English, and `1.2万` in
 * Chinese and in Japanese, which count in ten thousands.
 */
export function countOf(count: number, locale: string): string {
  return Intl.NumberFormat(locale, { notation: 'compact', maximumFractionDigits: 1 }).format(count);
}

/** How full the context of the model was at the last request, in percent, or null if its size is not known. */
export function contextPercent(lastInputTokens: number, window: number | null | undefined): number | null {
  if (!window || window <= 0) return null;
  return Math.max(0, Math.min(100, Math.round(lastInputTokens / window * 100)));
}

/**
 * What the service says a longer yes covers, in the words of the interface.
 *
 * The service finishes the sentence "do not ask again about ..." in one of
 * three ways (`permissions/__init__.py` in its sources). Any other way is
 * said in general words, so that a new one is never shown as it came.
 */
export function scopeOf(
  words: Messages,
  asked: { tool: string; target: string | null; standing: string | null },
): string | null {
  if (asked.standing === null) return null;
  const scope = words.hint.scope;
  if (asked.standing === 'this command, word for word') return scope.thisCommand;
  if (asked.standing === `any call of ${asked.tool}`) return scope.anyCall(asked.tool);
  if (asked.target !== null && asked.standing === `${asked.tool} on ${asked.target}`) {
    return scope.onTarget(asked.tool, asked.target);
  }
  return scope.other;
}

/**
 * What each preset sets: the mode of the sandbox and the policy of approval
 * (`permissions/__init__.py` in the sources of the service).
 */
export const presets: {
  id: PermissionPreset;
  name: 'readOnly' | 'workspaceWrite' | 'fullAccess';
  mode: string;
  policy: string;
}[] = [
  { id: 'read-only', name: 'readOnly', mode: 'read-only', policy: 'ask' },
  { id: 'workspace-write', name: 'workspaceWrite', mode: 'workspace-write', policy: 'ask' },
  { id: 'full-access', name: 'fullAccess', mode: 'danger-full-access', policy: 'never' },
];

/**
 * The preset that holds, or null where what holds is none of them.
 *
 * With full access a command runs without a question whatever the policy is,
 * so the policy does not tell that preset from anything.
 */
export function presetOf(mode: string | null, policy: string | null): PermissionPreset | null {
  if (mode === 'danger-full-access') return 'full-access';
  return presets.find(preset => preset.mode === mode && preset.policy === policy)?.id ?? null;
}

/** The arguments of a call, if they are an object of JSON. A call that was cut off may hold less. */
export function argumentsOf(item: ToolItem): Record<string, unknown> | null {
  try {
    const parsed: unknown = JSON.parse(item.arguments);
    return parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed)
      ? parsed as Record<string, unknown> : null;
  } catch {
    return null;
  }
}

/** The tools that run a command, whatever the shell is called on a system. */
export const commandTools = new Set(['bash', 'powershell']);

/** The kind of thing a tool does. The interface has a word and a picture for each. */
export type ToolKind = 'read' | 'write' | 'edit' | 'list' | 'search' | 'run' | 'todo' | 'skill' | 'subagent' | 'other';

export function toolKind(name: string): ToolKind {
  switch (name) {
    case 'read': return 'read';
    case 'write': return 'write';
    case 'edit': return 'edit';
    case 'glob': return 'list';
    case 'grep': return 'search';
    case 'todo_write': return 'todo';
    case 'skill': return 'skill';
    case 'subagent': return 'subagent';
    default: return commandTools.has(name) ? 'run' : 'other';
  }
}

/** What a tool does, in a word that fits before what it does it to. */
export function verbOf(words: Messages, name: string): string {
  const kind = toolKind(name);
  return kind === 'other' ? words.label.verb.other(name) : words.label.verb[kind];
}

/**
 * What a call acts on: the first of these arguments that it has, as the
 * protocol document says a terminal does it.
 */
export function objectOf(args: Record<string, unknown> | null): string {
  for (const key of ['command', 'path', 'pattern', 'description', 'name']) {
    const value = args?.[key];
    if (typeof value === 'string' && value.trim()) return value;
  }
  return '';
}

/** The last lines of a text, and how many lines before them were left out. */
export function tail(text: string, most: number): { shown: string; cut: number } {
  const lines = text.replace(/\n$/, '').split('\n');
  if (lines.length <= most) return { shown: lines.join('\n'), cut: 0 };
  return { shown: lines.slice(-most).join('\n'), cut: lines.length - most };
}
