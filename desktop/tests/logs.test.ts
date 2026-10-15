import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { logs, type LogName } from './logs';

// Where the repository keeps the logs that the tests of the interface replay copies of.
const originals: Record<LogName, string> = {
  'deepseek-read': 'tests/fixtures/recorded/deepseek-read/session.v1.jsonl',
  'deepseek-fix': 'tests/fixtures/recorded/deepseek-fix/session.v1.jsonl',
  tour: 'tests/fixtures/tour.jsonl',
};
const repository = new URL('../../', import.meta.url);

describe('the logs that the tests replay', () => {
  it('are the three that the repository keeps', () => {
    expect([...logs].sort()).toEqual(Object.keys(originals).sort());
  });

  it.each(logs)('%s is a copy of the one that the repository keeps, to the letter', (name) => {
    // A sample that is made anew and a copy that stays as it was tell two stories of one session.
    const copy = readFileSync(new URL(`./fixtures/${name}.jsonl`, import.meta.url), 'utf8');
    const original = readFileSync(new URL(originals[name], repository), 'utf8');
    expect(copy).toBe(original);
    expect(copy.length).toBeGreaterThan(1000);
  });
});
