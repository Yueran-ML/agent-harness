// @vitest-environment jsdom
/**
 * The trace of a task, as the page draws it: its steps side by side, the switches of its kinds,
 * its search, its rows turn by turn, and what a row holds once it is opened.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TraceView } from '../src/app/TraceView';
import type { SessionEvent } from '../src/protocol';
import { begin, call, end, likeABrowser, log, user, workspaceWith, type Written } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const spent = (input: number, output: number) => ({ input_tokens: input, output_tokens: output, cache_read_tokens: 0, cache_write_tokens: 0 });
const answer = (words: string, thought = ''): Written => ['assistant/message', {
  message: {
    role: 'assistant',
    content: [...(thought ? [{ type: 'reasoning', text: thought }] : []), ...(words ? [{ type: 'text', text: words }] : [])],
  },
  usage: spent(12_000, 320), finish_reason: 'stop',
}];
const step = (turn: number, number: number): Written => ['step/start', { turn, step: number }];
const done = (id: string, words: string): Written => ['tool/result', { message: { role: 'tool', call_id: id, content: [{ type: 'text', text: words }] }, error: null }];
const wrong = (id: string, code: string, reason: string): Written =>
  ['tool/result', { message: { role: 'tool', call_id: id, content: [{ type: 'text', text: reason }] }, error: { code, reason } }];

/** A task of two turns: three steps that read and search, one call that fails, and a second turn of one step. */
const task = (): SessionEvent[] => log(
  begin(1), user('Fix the parser.'),
  step(1, 1), answer('Let me look.', 'The parser is in src.'), call('c1', 'read', { path: 'src/parser.py' }), done('c1', 'def parse(): ...'),
  step(1, 2), answer(''), call('c2', 'read', { path: 'missing.md' }), wrong('c2', 'NOT_FOUND', 'There is no file missing.md.'),
  call('c3', 'grep', { pattern: 'parse' }), done('c3', 'src/parser.py:3'),
  step(1, 3), answer('It is fixed.'),
  end(1),
  begin(2), user('Now the tests.'), step(2, 1), answer('They pass.'), end(2),
);

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" on the page.`);
  return found;
};
const drawn = (events = task()) => render(<TraceView workspace={workspaceWith({ events })} locale="en" />);
/** The rows as they are shown: kind and what they say, with the end of the row after a bar. */
const rows = () => marked('trace-row').map(row => [row.dataset.kind, row.querySelector('.trace-main')!.textContent, row.querySelector('.trace-meta')?.textContent ?? '']);
const kind = (name: string) => marked('trace-kind').find(each => each.dataset.kind === name)!;

describe('the trace of a task', () => {
  it('shows the steps side by side, and turn by turn what was said, answered and called', () => {
    drawn();
    expect(marked('trace-column')).toHaveLength(4);
    expect(marked('trace-turn').map(turn => turn.querySelector('.trace-turn-head')!.textContent)).toEqual([
      // A tenth of a second from one event to the next, as the log of the tests is written.
      'Turn 13 steps · 1.4 s · 37K tokens · done',
      'Turn 21 step · 0.4 s · 12.3K tokens · done',
    ]);
    // What the model thought is left out at first.
    expect(rows()).toEqual([
      ['input', 'Fix the parser.', ''],
      ['reply', 'Let me look.', '0.1 s · 12K in · 320 out'],
      ['tool', 'read{"path":"src/parser.py"}→ def parse(): ...', '0.1 s'],
      // An answer that only calls tools says how many.
      ['reply', 'Calls 2 tools', '0.1 s · 12K in · 320 out'],
      ['tool', 'read{"path":"missing.md"}→ NOT_FOUND There is no file missing.md.', '0.1 s'],
      ['tool', 'grep{"pattern":"parse"}→ src/parser.py:3', '0.1 s'],
      ['reply', 'It is fixed.', '0.1 s · 12K in · 320 out'],
      ['input', 'Now the tests.', ''],
      ['reply', 'They pass.', '0.1 s · 12K in · 320 out'],
    ]);
    expect(marked('trace-row').filter(row => row.dataset.wrong !== undefined).map(row => row.dataset.kind)).toEqual(['tool']);
  });

  it('shows what the model thought when it is asked for, and leaves out a kind that is switched off', () => {
    drawn();
    expect(kind('thinking').getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(kind('thinking'));
    expect(rows()[1]).toEqual(['thinking', 'The parser is in src.', '']);
    fireEvent.click(kind('tool'));
    expect(rows().map(([each]) => each)).toEqual(['input', 'thinking', 'reply', 'reply', 'reply', 'input', 'reply']);
    expect(kind('tool').getAttribute('aria-pressed')).toBe('false');
  });

  it('shows only what went wrong, where that is asked for', () => {
    drawn();
    fireEvent.click(one('trace-wrong'));
    expect(one('trace-wrong').getAttribute('aria-pressed')).toBe('true');
    expect(rows().map(([each, said]) => [each, said])).toEqual([['tool', 'read{"path":"missing.md"}→ NOT_FOUND There is no file missing.md.']]);
  });

  it('says nothing of a time too short to be worth saying', () => {
    const quick = task().map(event => (event.type === 'tool/result' || event.type === 'assistant/message' ? { ...event, time: event.time - 70 } : event));
    drawn(quick);
    expect(rows().slice(1, 3)).toEqual([
      ['reply', 'Let me look.', '12K in · 320 out'],
      ['tool', 'read{"path":"src/parser.py"}→ def parse(): ...', ''],
    ]);
  });

  it('opens a row to show all that it holds, and copies it', async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } });
    drawn();
    const failed = marked('trace-row')[4];
    fireEvent.click(failed);
    expect(failed.getAttribute('aria-expanded')).toBe('true');
    const detail = failed.parentElement!.querySelector('.trace-detail')!;
    expect([...detail.querySelectorAll('.trace-block-head')].map(head => head.textContent)).toEqual(['Arguments', 'Error']);
    // The arguments laid out to be read; the result only said again what the error says.
    expect([...detail.querySelectorAll('pre')].map(block => block.textContent)).toEqual([
      '{\n  "path": "missing.md"\n}', 'NOT_FOUND\nThere is no file missing.md.',
    ]);
    const copy = detail.querySelector<HTMLButtonElement>('.trace-copy')!;
    expect(copy.getAttribute('aria-label')).toBe('Copy');
    await act(async () => { fireEvent.click(copy); });
    expect(writeText).toHaveBeenCalledWith('{\n  "path": "missing.md"\n}');
    expect(copy.getAttribute('aria-label')).toBe('Copied');
    // An answer shows its whole text.
    fireEvent.click(marked('trace-row')[1]);
    expect(marked('trace-row')[1].parentElement!.querySelector('pre')!.textContent).toBe('Let me look.');
    fireEvent.click(failed);
    expect(failed.getAttribute('aria-expanded')).toBe('false');
  });

  it('looks through the rows, and goes from one find to the next with the keyboard staying in the search', () => {
    vi.useFakeTimers();
    drawn();
    const search = one('trace-search') as HTMLInputElement;
    search.focus();
    fireEvent.change(search, { target: { value: 'READ' } });
    // Enter finds what is written even before the search has looked.
    fireEvent.keyDown(search, { key: 'Enter' });
    expect(one('trace-count').textContent).toBe('1 of 2');
    expect(document.querySelector('[data-marked]')!.textContent).toContain('src/parser.py');
    fireEvent.keyDown(search, { key: 'Enter' });
    expect(one('trace-count').textContent).toBe('2 of 2');
    fireEvent.keyDown(search, { key: 'Enter', shiftKey: true });
    expect(one('trace-count').textContent).toBe('1 of 2');
    expect(document.activeElement).toBe(search);
    expect(marked('trace-row').filter(row => row.dataset.found !== undefined)).toHaveLength(2);
    fireEvent.change(search, { target: { value: 'nowhere' } });
    act(() => { vi.advanceTimersByTime(100); });
    expect(one('trace-count').textContent).toBe('Nothing found');
    // Esc empties the search.
    fireEvent.keyDown(search, { key: 'Escape' });
    expect(search.value).toBe('');
    expect(marked('trace-count')).toEqual([]);
  });

  it('leads from a step in the overview to its first row, and unfolds its turn for it', () => {
    const seen: Element[] = [];
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) { seen.push(this); };
    drawn();
    fireEvent.click(marked('trace-turn')[1].querySelector('.trace-turn-head')!);
    expect(marked('trace-turn')[1].querySelectorAll('[data-test="trace-row"]')).toHaveLength(0);
    fireEvent.click(marked('trace-column')[3]);
    const row = document.querySelector<HTMLElement>('[data-marked]')!;
    expect(row.textContent).toContain('Now the tests.');
    expect(seen.at(-1)).toBe(row);
    expect(document.activeElement).toBe(row);
    // Having left the end, the way back to it is offered.
    expect(marked('trace-latest')).toHaveLength(1);
  });

  it('leads from a step to the first of its rows that is shown, and shows a kind only where none of its rows is', () => {
    // The answer of the third step comes after what the model thought, which is not shown.
    const [last] = answer('It is fixed.', 'Done thinking.').slice(1) as [Record<string, unknown>];
    drawn(task().map(event => (event.seq === 13 ? { ...event, data: last } : event)));
    fireEvent.click(marked('trace-column')[2]);
    expect(document.querySelector<HTMLElement>('[data-marked]')!.textContent).toContain('It is fixed.');
    expect(kind('thinking').getAttribute('aria-pressed')).toBe('false');
    // Where every row of the step is left out, the step shows its first row and the kind of it.
    fireEvent.click(kind('reply'));
    fireEvent.click(marked('trace-column')[2]);
    expect(document.querySelector<HTMLElement>('[data-marked]')!.textContent).toContain('Done thinking.');
    expect(kind('thinking').getAttribute('aria-pressed')).toBe('true');
    // Where only what went wrong is shown, a step without anything that did shows the rest again.
    fireEvent.click(one('trace-wrong'));
    fireEvent.click(marked('trace-column')[3]);
    expect(one('trace-wrong').getAttribute('aria-pressed')).toBe('false');
    expect(document.querySelector<HTMLElement>('[data-marked]')!.textContent).toContain('Now the tests.');
  });

  it('moves through the steps of the overview with the keyboard', () => {
    drawn();
    const overview = one('trace-overview').querySelector('svg')!;
    fireEvent.keyDown(overview, { key: 'Home' });
    fireEvent.keyDown(overview, { key: 'ArrowRight' });
    expect(marked('trace-column').map(column => column.dataset.at !== undefined)).toEqual([false, true, false, false]);
    expect(overview.getAttribute('aria-describedby')).toBe('trace-step-1');
    fireEvent.keyDown(overview, { key: 'End' });
    // The arrows stop at the ends.
    fireEvent.keyDown(overview, { key: 'ArrowRight' });
    expect(overview.getAttribute('aria-describedby')).toBe('trace-step-3');
    fireEvent.keyDown(overview, { key: 'ArrowLeft' });
    fireEvent.keyDown(overview, { key: 'Enter' });
    expect(document.querySelector('[data-marked]')!.textContent).toContain('It is fixed.');
  });

  it('moves from row to row with the arrows, and goes to the search with a slash', () => {
    drawn();
    const list = one('trace-scroll');
    const first = marked('trace-row')[0];
    first.focus();
    fireEvent.keyDown(first, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(marked('trace-row')[1]);
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(first);
    fireEvent.keyDown(list, { key: '/' });
    expect(document.activeElement).toBe(one('trace-search'));
  });

  it('tells each step of the overview in words', () => {
    drawn();
    expect(marked('trace-column').map(column => column.getAttribute('aria-label'))).toEqual([
      'Turn 1, step 1. The model took 0.1 s. One call.',
      'Turn 1, step 2. The model took 0.1 s. 2 calls, 1 of them went wrong.',
      'Turn 1, step 3. The model took 0.1 s.',
      'Turn 2, step 1. The model took 0.1 s.',
    ]);
  });

  it('says that nothing has happened in a task without rows', () => {
    drawn([]);
    expect(one('trace-empty').textContent).toBe('Nothing has happened in this task yet.');
    expect(marked('trace')).toEqual([]);
  });
});
