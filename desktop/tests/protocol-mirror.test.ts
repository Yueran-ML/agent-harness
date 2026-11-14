/**
 * That the TS mirror (`src/studio/engine.ts`) has what `engine/PROTOCOL.md` lists (plan of phase
 * 3, T17): every method is one that the page calls (`PageMethods`), or is called by the main
 * process of Electron, or is named below with why the desktop does not call it; every
 * notification has its params (`EngineNotifications`); and every operation that the engine may
 * tell in `done` is an `Op` that `applyOp` applies.
 */
import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');
const protocol = read('../../engine/PROTOCOL.md');
const mirror = read('../src/studio/engine.ts');
const electron = readdirSync(new URL('../electron/', import.meta.url))
  .filter(name => name.endsWith('.cjs') || name.endsWith('.mjs'))
  .map(name => read(`../electron/${name}`))
  .join('\n');

/** The cells of a row of a table of Markdown, a `\|` kept in its cell. */
function cells(line: string): string[] | null {
  const row = line.trim();
  if (!row.startsWith('|') || !row.endsWith('|')) return null;
  return row.slice(1, -1).split(/(?<!\\)\|/).map(cell => cell.trim());
}

/** The names in the first column of the tables whose header is `header`. */
function named(text: string, header: string[]): Set<string> {
  const names = new Set<string>();
  const lines = text.split('\n');
  for (let at = 0; at < lines.length; at++) {
    const head = cells(lines[at]);
    if (!head || head.length !== header.length || head.some((cell, i) => cell !== header[i])) continue;
    for (at += 2; at < lines.length && cells(lines[at]); at++) {
      for (const match of cells(lines[at])![0].matchAll(/`([^`]+)`/g)) names.add(match[1]);
    }
  }
  return names;
}

/** The keys of an interface of the mirror, written `'a.b': …` on lines of their own. */
function keysOf(name: string): Set<string> {
  const start = mirror.indexOf(`export interface ${name} {`);
  expect(start, name).toBeGreaterThan(-1);
  const body = mirror.slice(start, mirror.indexOf('\n}', start));
  return new Set([...body.matchAll(/^ {2}'([^']+)':/gm)].map(match => match[1]));
}

const methods = [...named(protocol, ['method', 'params', 'result'])].filter(name => !name.endsWith('.*'));
const notifications = named(protocol, ['method', 'params']);

/** Methods that the main process of Electron calls, never the page. */
const OF_THE_MAIN_PROCESS = [
  'hello', 'engine.yield', 'view.attach', 'media.import', 'still.import', 'still.export', 'grade.export_lut', 'render.start',
];

/** Methods that the desktop does not call, and why. */
const NOT_CALLED: Record<string, string> = {
  'media.inspect': 'the migration of older projects by the agent (Python) reads media so',
  'transport.frame_digest': 'a hook of the tests of the engine (`--test-hooks`)',
  'view.detach': 'the view goes with its window',
};

describe('the TS mirror of the protocol', () => {
  it('reads the tables of PROTOCOL.md', () => {
    expect(methods).toContain('automation.punch');
    expect(methods).toContain('transport.scrub_audio');
    expect(methods).toEqual(expect.arrayContaining(['transport.loop', 'transport.play_range']));
    expect(methods).not.toContain('transport.*');
    expect(notifications).toContain('automation.written');
    expect(cells('| `trim` | `edge:in\\|out` |')).toEqual(['`trim`', '`edge:in\\|out`']);
  });

  it('has every method that the protocol lists, or says why the page does not call it', () => {
    const page = keysOf('PageMethods');
    const left = methods.filter(method => !page.has(method) && !OF_THE_MAIN_PROCESS.includes(method) && !(method in NOT_CALLED));
    expect(left).toEqual([]);
    for (const method of OF_THE_MAIN_PROCESS) {
      expect(methods, method).toContain(method);
      expect(electron, method).toContain(`'${method}'`);
      expect(page.has(method), method).toBe(false);
    }
    for (const method of Object.keys(NOT_CALLED)) {
      expect(methods, method).toContain(method);
      expect(page.has(method) || electron.includes(`'${method}'`), method).toBe(false);
    }
  });

  it('calls no method that the protocol does not list', () => {
    expect([...keysOf('PageMethods')].filter(method => !methods.includes(method))).toEqual([]);
  });

  it('has the params of every notification, and of none that is not one', () => {
    expect([...keysOf('EngineNotifications')].sort()).toEqual([...notifications].sort());
  });

  it('applies every operation of the protocol', () => {
    const ops = new Set([
      ...named(protocol, ['op', 'fields']),
      ...named(protocol, ['op', 'fields', 'inverse']),
    ]);
    expect(ops).toContain('set_media_streams');
    const union = new Set([...mirror.matchAll(/\{ op: '([a-z_]+)'/g)].map(match => match[1]));
    const applied = new Set([...mirror.matchAll(/case '([a-z_]+)':/g)].map(match => match[1]));
    expect([...ops].filter(op => !union.has(op))).toEqual([]);
    expect([...ops].filter(op => !applied.has(op))).toEqual([]);
    expect([...union].filter(op => !ops.has(op))).toEqual([]);
  });
});
