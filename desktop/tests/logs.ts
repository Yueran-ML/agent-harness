import { readFileSync } from 'node:fs';
import type { SessionEvent } from '../src/protocol';

/**
 * The session logs that the tests replay. Two are real runs against DeepSeek.
 * The third is the sample of the protocol, in which every kind of event occurs.
 * They are copies: `tests/fixtures/recorded` and `docs/protocol/samples` at the
 * root of the repository hold the originals.
 */
export const logs = ['deepseek-read', 'deepseek-fix', 'tour'] as const;
export type LogName = typeof logs[number];

/** The events of a log. Its first line is the header of the session, and is left out. */
export function eventsOf(name: LogName): SessionEvent[] {
  const text = readFileSync(new URL(`./fixtures/${name}.jsonl`, import.meta.url), 'utf8');
  return text
    .split(/\r?\n/)
    .filter(line => line.trim())
    .slice(1)
    .map(line => JSON.parse(line) as SessionEvent);
}
