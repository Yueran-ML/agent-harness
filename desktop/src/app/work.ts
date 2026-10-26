/**
 * What the shell answers of the tasks that run at set times, of the worktrees of a project and of
 * the groups of the assistant in QQ, as the page knows them; and how a time is said. Nothing here
 * draws.
 */
import type { Messages } from '../messages/en';

export type Every = 'minutes' | 'hour' | 'day' | 'week';
export const WEEKDAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'] as const;
export type Weekday = typeof WEEKDAYS[number];

/** A task that runs at set times. */
export interface Schedule {
  id: string;
  name: string;
  cwd: string;
  prompt: string;
  every: Every;
  minutes: number | null;
  at: string | null;
  weekday: Weekday | null;
  missed: 'run' | 'skip';
  enabled: boolean;
  /** When it ran last, in milliseconds since 1970, and how that ended. */
  last: number | null;
  lastEnd: string | null;
}

/** The worktree of a task in a project. */
export interface Worktree {
  name: string;
  path: string;
  branch: string;
}

/** A group of QQ that the bot was asked in. */
export interface Group {
  id: string;
  name: string | null;
  /** How many characters its memory holds, and how many it may hold. */
  memory: number;
  most: number;
}

type Record_ = Record<string, unknown>;
const record = (value: unknown): Record_ =>
  value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record_ : {};
const text = (value: unknown): string | null => (typeof value === 'string' ? value : null);
const whole = (value: unknown): number | null => (typeof value === 'number' && Number.isInteger(value) ? value : null);
const list = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);

/** Read the tasks that `columbina schedules list --json` answers. What it does not say is left out. */
export function schedulesOf(answer: unknown): Schedule[] {
  const found: Schedule[] = [];
  for (const each of list(record(answer).schedules)) {
    const one = record(each);
    const id = text(one.id);
    const name = text(one.name);
    const cwd = text(one.cwd);
    const every = (['minutes', 'hour', 'day', 'week'] as const).find(kind => kind === one.every);
    if (id === null || name === null || cwd === null || every === undefined) continue;
    found.push({
      id, name, cwd, every,
      prompt: text(one.prompt) ?? '',
      minutes: whole(one.minutes),
      at: text(one.at),
      weekday: WEEKDAYS.find(day => day === one.weekday) ?? null,
      missed: one.missed === 'skip' ? 'skip' : 'run',
      enabled: one.enabled !== false,
      last: whole(one.last),
      lastEnd: text(one.last_end),
    });
  }
  return found;
}

/** Read the worktrees that `columbina worktrees list --json` answers. */
export function worktreesOf(answer: unknown): Worktree[] {
  const found: Worktree[] = [];
  for (const each of list(record(answer).worktrees)) {
    const one = record(each);
    const name = text(one.name);
    const path = text(one.path);
    const branch = text(one.branch);
    if (name !== null && path !== null && branch !== null) found.push({ name, path, branch });
  }
  return found;
}

/** Read the groups that `columbina qq --groups --json` answers. */
export function groupsOf(answer: unknown): Group[] {
  const found: Group[] = [];
  for (const each of list(record(answer).groups)) {
    const one = record(each);
    const id = text(one.id);
    if (id === null) continue;
    found.push({ id, name: text(one.name), memory: whole(one.memory) ?? 0, most: whole(one.most) ?? 0 });
  }
  return found;
}

/** Say in a few words when a task runs. */
export function whenOf(words: Messages, schedule: Schedule): string {
  const when = words.label.work.schedules.when;
  if (schedule.every === 'minutes') return when.minutes(schedule.minutes ?? 0);
  if (schedule.every === 'hour') return when.hour((schedule.at ?? '').replace(':', ''));
  if (schedule.every === 'day') return when.day(schedule.at ?? '');
  return when.week(schedule.weekday ?? 'mon', schedule.at ?? '');
}

/**
 * What a task is to be added with, from what the form holds, or null where it holds no such
 * task: a name, a folder, a prompt, and the time that its kind needs.
 */
export function taskOf(form: {
  name: string; cwd: string; prompt: string; every: Every; minutes: string; time: string; minute: string;
  weekday: Weekday; missed: 'run' | 'skip';
}): {
  name: string; cwd: string; prompt: string; every: Every; minutes?: number; at?: string; weekday?: Weekday;
  missed: 'run' | 'skip';
} | null {
  const name = form.name.trim();
  const prompt = form.prompt.trim();
  if (name === '' || prompt === '' || form.cwd === '') return null;
  const base = { name, cwd: form.cwd, prompt, every: form.every, missed: form.missed };
  if (form.every === 'minutes') {
    const minutes = Number(form.minutes);
    return Number.isInteger(minutes) && minutes >= 5 ? { ...base, minutes } : null;
  }
  if (form.every === 'hour') {
    const minute = Number(form.minute);
    return Number.isInteger(minute) && minute >= 0 && minute < 60 ? { ...base, at: `:${String(minute).padStart(2, '0')}` } : null;
  }
  if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(form.time)) return null;
  return form.every === 'day' ? { ...base, at: form.time } : { ...base, at: form.time, weekday: form.weekday };
}

/** The message of what failed, as the shell said it. */
export function failureOf(problem: unknown): string {
  const said = problem instanceof Error ? problem.message : String(problem);
  // What a handler of the shell threw comes with the name of the channel before it.
  return said.replace(/^Error invoking remote method '[^']*': (Error: )?/, '');
}
