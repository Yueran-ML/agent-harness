/**
 * What the interface works out before it says something: the name of a
 * folder, how long ago, what a call acts on. Nothing here draws anything, so
 * all of it can be tried with plain values.
 */
import { serviceCalled, type Listed, type ListedService } from '../key/form';
import type { Messages } from '../messages/en';
import type { SkillSummary } from '../protocol';
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
 * What people call the service that a model comes from, or null where no service is named.
 *
 * Args:
 *   provider: The name of the service, as the service process or its log gives it.
 *   listed: The services that are set up, where the window has the list.
 */
export function serviceTitle(provider: string | null | undefined, listed: Listed | null): string | null {
  return provider ? serviceCalled(provider, listed) : null;
}

/**
 * The service in use, and the services that the settings tell of.
 *
 * In use is the service that was chosen. Where none was chosen the program goes by its
 * profile, and the service in use is the one that the service process names. The settings
 * tell of that one and of every service that the user has had to do with: one of their own,
 * or one that a key is there for. The others are in the window that sets a service up.
 *
 * Args:
 *   listed: The services that are set up, where the window has the list.
 *   provider: The name of the service that the service process talks to, where one runs.
 */
export function servicesToTell(
  listed: Listed | null, provider: string | null | undefined,
): { inUse: string | null; known: ListedService[] } {
  const inUse = listed?.use || provider || null;
  const known = listed?.services.filter(service => service.name === inUse || service.own || service.key !== 'none') ?? [];
  return { inUse, known };
}

/**
 * Whether the next request is not known to fail for want of a key.
 *
 * A turn that failed for its key is worth trying again then, and not before. The service
 * process looks for the key with every request, so a key that was stored since is found
 * without a new start. Only of a service that is known by name and asks for a key can the
 * window tell that a request will fail: one of the user's own is asked without a key where
 * none is stored, and so is one that the list does not have. A window without a list of
 * services knows nothing, and offers nothing.
 *
 * Args:
 *   provider: The name of the service that the service process talks to.
 *   listed: The services that are set up, where the window has the list.
 */
export function keyAtHand(provider: string | null | undefined, listed: Listed | null): boolean {
  if (!provider || listed === null) return false;
  const service = listed.services.find(candidate => candidate.name === provider);
  if (!service || service.own || service.key_env === null) return true;
  return service.key === 'stored' || service.key === 'environment';
}

/**
 * Read what the service process answers when it is asked for the skills of a session.
 *
 * Whatever is no list of skills is none. Of a skill that says nothing of who may load it,
 * both may: that is what a skill is unless its file says otherwise.
 */
export function skillsOf(given: unknown): SkillSummary[] {
  const listed = (given as { skills?: unknown } | null)?.skills;
  if (!Array.isArray(listed)) return [];
  const read: SkillSummary[] = [];
  for (const one of listed as unknown[]) {
    if (one === null || typeof one !== 'object') continue;
    const said = one as Record<string, unknown>;
    if (typeof said.name !== 'string' || said.name === '') continue;
    read.push({
      name: said.name,
      description: typeof said.description === 'string' ? said.description : '',
      user_invocable: said.user_invocable !== false,
      model_invocable: said.model_invocable !== false,
    });
  }
  return read;
}

/** How many skills the page of a new task names. The command `/skills` lists them all. */
export const SKILLS_SHOWN = 6;

/**
 * The skills that the page of a new task names, and how many more there are.
 *
 * Those that a person can hand over stand first, because a click on one of them does
 * something. Among each kind the order of the service process is kept, which is by name.
 */
export function skillsToShow(skills: readonly SkillSummary[], most = SKILLS_SHOWN): { shown: SkillSummary[]; more: number } {
  const ordered = [...skills.filter(skill => skill.user_invocable), ...skills.filter(skill => !skill.user_invocable)];
  return { shown: ordered.slice(0, most), more: Math.max(0, ordered.length - most) };
}

/**
 * The name of the skill that a message hands to the agent, or null where it names none.
 *
 * The service process writes the name at the head of such a message, on a line of its own.
 */
export function skillNamed(text: string): string | null {
  const found = /^Skill: (.+)$/m.exec(text);
  return found ? found[1].trim() || null : null;
}

/** What stands in the field after a skill was chosen: the command that hands it over, ready for the task. */
export function skillLine(name: string): string {
  return `/skill ${name} `;
}

/** How the task that is open stands, in a word and in the tone that goes with it. */
export interface Standing {
  words: string;
  /** At work, at rest, or of no use until something is done about it. */
  tone: 'live' | 'rest' | 'off';
}

/**
 * Say how the task that is open stands.
 *
 * Args:
 *   words: The catalogue.
 *   at: What is known of it. `status` is what the service process says of the task, `greeted`
 *     whether that process has answered at all, and `cancelling` whether the user has asked
 *     for the turn to be stopped and it is not stopped yet.
 */
export function standingOf(words: Messages, at: {
  status: string | null | undefined; cancelling: boolean; loading: boolean; greeted: boolean; connected: boolean;
}): Standing {
  const said = at.cancelling ? words.label.status.stopping
    : at.status === 'running' ? words.label.status.working
      : at.status === 'stopped' ? words.label.status.stopped
        : at.loading ? (at.greeted ? words.label.status.opening : words.label.status.starting)
          : !at.connected ? words.label.status.offline : words.label.status.ready;
  const tone = at.status === 'running' ? 'live'
    : at.status === 'stopped' || (!at.connected && !at.loading) ? 'off' : 'rest';
  return { words: said, tone };
}

/** How long a start may take before more is said of it, in milliseconds. */
export const STILL_STARTING_MS = 3000;

/** How many events of a log are asked for at once. A log of more than that is a long one. */
export const HISTORY_PAGE = 500;

/**
 * What is said while the page waits for the service: that it is being started, that a task
 * is being opened, or how far the log of a long task is read.
 */
export function waitingWords(
  words: Messages, at: { answered: boolean; folder: string | null; reading: { read: number; of: number } | null },
): string {
  if (at.reading) return words.hint.reading(at.reading.read, at.reading.of);
  return at.answered || !at.folder ? words.hint.openingTask : words.hint.starting(at.folder);
}

/**
 * What is said of a service that is gone. One that never answered did not end:
 * it could not be started.
 */
export function goneWords(words: Messages, answered: boolean): string {
  return answered ? words.hint.serviceEnded : words.hint.serviceNotStarted;
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

/** The kind of subagent that a call hands its task to, or null where it names none or hands nothing over. */
export function subagentKind(name: string, args: Record<string, unknown> | null): string | null {
  if (toolKind(name) !== 'subagent') return null;
  const kind = args?.type;
  return typeof kind === 'string' && kind.trim() ? kind.trim() : null;
}

/** From this long on, a call says how long it runs or ran. A shorter time is not worth a number. */
export const TIME_WORTH_SAYING_MS = 1000;

/**
 * How long a call ran or has run so far, in seconds, or null where that is not worth saying.
 *
 * A call that is through says the time that it took. One that runs says the whole seconds
 * that have gone by, so that its number moves once a second and no more often.
 *
 * Args:
 *   item: The call.
 *   running: Whether the call runs at this moment. One that waits for an answer does not.
 *   now: This moment, in milliseconds since 1970.
 */
export function callSeconds(item: ToolItem, running: boolean, now: number): number | null {
  if (item.durationMs !== null) return item.durationMs >= TIME_WORTH_SAYING_MS ? item.durationMs / 1000 : null;
  if (!running) return null;
  const gone = now - item.beganAt;
  return gone >= TIME_WORTH_SAYING_MS ? Math.floor(gone / 1000) : null;
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

/**
 * The last lines of a text, and how many lines before them were left out.
 *
 * Args:
 *   text: The text. A line break at its very end begins no line.
 *   most: How many lines are shown at most, or null for all of them.
 */
export function tail(text: string, most: number | null): { shown: string; cut: number } {
  const lines = text.replace(/\n$/, '').split('\n');
  if (most === null || lines.length <= most) return { shown: lines.join('\n'), cut: 0 };
  return { shown: lines.slice(-most).join('\n'), cut: lines.length - most };
}
