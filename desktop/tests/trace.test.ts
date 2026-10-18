/**
 * What a task did over time, as the page reads it from its log: how fast the model wrote.
 */
import { describe, expect, it } from 'vitest';
import { holds, speedOf, traceOf, wentWrong, wordsOf, type TraceRow } from '../src/app/trace';
import type { SessionEvent } from '../src/protocol';
import { projectEvents } from '../src/state';
import { begin, call, end, failed, log, result, user, type Written } from './mounted';

/** A log, each event at the time that is given, in milliseconds. */
const at = (...given: [time: number, type: string, data?: Record<string, unknown>][]): SessionEvent[] =>
  given.map(([time, type, data = {}], seq) => ({ seq, type, data, time }));
const answered = (written: number, finish_reason: string | null = 'stop') => ({
  message: { role: 'assistant', content: [{ type: 'text', text: 'Done.' }] },
  usage: { input_tokens: 900, output_tokens: written, cache_read_tokens: 0, cache_write_tokens: 0 },
  finish_reason,
});
const failure = (written = 0) => ({
  message: { role: 'assistant', content: [] },
  usage: { input_tokens: 900, output_tokens: written, cache_read_tokens: 0, cache_write_tokens: 0 },
  failure: { code: 'RATE_LIMITED', message: 'slow down' },
});

describe('how fast the model wrote', () => {
  it('is what the answers wrote over the time from when each request was made to when its answer was in', () => {
    expect(speedOf(at(
      [0, 'turn/start', { turn: 1 }],
      [1_000, 'step/start', { turn: 1, step: 1 }],
      [3_000, 'assistant/message', answered(100)],
      // The calls of a step take no time of the model.
      [3_000, 'tool/call', { call_id: 'c1', name: 'read', arguments: '{}' }],
      [9_000, 'tool/result', { message: { role: 'tool', call_id: 'c1', content: [] }, error: null }],
      [9_000, 'step/start', { turn: 1, step: 2 }],
      [11_000, 'assistant/message', answered(300)],
    ))).toBe(400 / 4);
  });

  it('counts from when a request is made again, after one that failed and the time to wait', () => {
    expect(speedOf(at(
      [0, 'step/start', { turn: 1, step: 1 }],
      [5_000, 'assistant/attempt', failure(20)],
      [5_000, 'llm/retry', { attempt: 1, code: 'RATE_LIMITED', delay_ms: 2_000 }],
      [9_000, 'assistant/message', answered(200)],
    ))).toBe(200 / 2);
    // Without a time to wait, the request is made again as soon as the one before failed.
    expect(speedOf(at(
      [0, 'step/start', { turn: 1, step: 1 }],
      [5_000, 'assistant/attempt', failure()],
      [7_000, 'assistant/message', answered(100)],
    ))).toBe(100 / 2);
  });

  it('counts from the end of a compaction within a step', () => {
    expect(speedOf(at(
      [0, 'step/start', { turn: 1, step: 1 }],
      [100, 'compaction/start', {}],
      [6_000, 'compaction/end', { outcome: {} }],
      [8_000, 'assistant/message', answered(100)],
    ))).toBe(100 / 2);
  });

  it('leaves out answers that were cut short or wrote nothing, and what came outside of a step', () => {
    expect(speedOf(at(
      [0, 'step/start', { turn: 1, step: 1 }],
      [1_000, 'assistant/message', answered(500, null)],
      [1_000, 'step/start', { turn: 1, step: 2 }],
      [2_000, 'assistant/message', answered(0)],
      // Neither a retry nor a compaction makes a request where no step is under way.
      [2_000, 'llm/retry', { delay_ms: 1_000 }],
      [2_500, 'compaction/end', { outcome: {} }],
      [3_000, 'assistant/message', answered(50)],
      [4_000, 'step/start', { turn: 1, step: 3 }],
      [6_000, 'assistant/message', answered(100)],
    ))).toBe(100 / 2);
  });

  it('is none where no answer tells anything', () => {
    expect(speedOf([])).toBeNull();
    expect(speedOf(at([0, 'step/start', { turn: 1, step: 1 }], [0, 'assistant/message', answered(100)]))).toBeNull();
    // A time to wait that the log gives in no way that can be read is none.
    expect(speedOf(at(
      [0, 'step/start', { turn: 1, step: 1 }],
      [1_000, 'llm/retry', { delay_ms: 'soon' }],
      [3_000, 'assistant/message', answered(100)],
    ))).toBe(100 / 2);
  });
});

describe('the trace of a task', () => {
  const spent = (input: number, output: number, cached = 0) => ({
    input_tokens: input, output_tokens: output, cache_read_tokens: cached, cache_write_tokens: 0,
  });
  /** An answer of the model, with what it thought, as the log is written. */
  const answer = (words: string, thought = '', usage = spent(100, 10), finish_reason: string | null = 'stop'): Written =>
    ['assistant/message', {
      message: {
        role: 'assistant',
        content: [...(thought ? [{ type: 'reasoning', text: thought }] : []), ...(words ? [{ type: 'text', text: words }] : [])],
      },
      usage, finish_reason,
    }];
  const step = (turn: number, number: number): Written => ['step/start', { turn, step: number }];
  const traced = (events: SessionEvent[]) => traceOf(events, projectEvents(events));
  const kinds = (trace: ReturnType<typeof traced>) => trace.rows.map(row => `${row.kind}@${row.step}`);

  it('gives every step its answer, its calls and whether input came in for it', () => {
    const events = log(
      begin(1), user('Fix the parser.'),
      step(1, 1), answer('Let me look.', 'The parser is in src.', spent(1_000, 20, 9_000)),
      call('c1', 'read', { path: 'src/parser.py' }), result('c1', 'def parse(): ...'),
      step(1, 2), answer('', '', spent(500, 5)), call('c2', 'edit', { path: 'src/parser.py' }), result('c2', 'Edited.'),
      step(1, 3), answer('It is fixed.'),
      end(1),
    );
    const trace = traced(events);
    expect(trace.steps.map(each => [each.turn, each.step, each.input, each.calls, each.failed])).toEqual([
      [1, 1, true, ['done'], 0], [1, 2, false, ['done'], 0], [1, 3, false, [], 0],
    ]);
    // Each answer took a tenth of a second, from the beginning of its step.
    expect(trace.steps.map(each => each.answer?.tookMs)).toEqual([100, 100, 100]);
    expect(trace.steps[0].answer?.usage).toEqual(spent(1_000, 20, 9_000));
    // An answer that only calls a tool is a row as well, so that what it took is shown.
    expect(kinds(trace)).toEqual([
      'input@0', 'thinking@0', 'reply@0', 'tool@0', 'reply@1', 'tool@1', 'reply@2',
    ]);
    // What was said before the first step is the first row of that step.
    expect(trace.steps.map(each => each.row)).toEqual(['event-1', 'event-7', 'event-11']);
    expect(trace.rows.find(row => row.id === 'event-3-thinking')).toMatchObject({ kind: 'thinking', text: 'The parser is in src.' });
  });

  it('tells each turn: its steps, how it ended, its tokens and its first row', () => {
    const events = log(
      begin(1), user('One.'), step(1, 1), answer('Done.', '', spent(100, 10, 50)), ['request/aside', { usage: spent(5, 1) }], end(1),
      begin(2), user('Two.'), step(2, 1),
      ['assistant/attempt', { message: { role: 'assistant', content: [] }, usage: spent(30, 3), failure: { code: 'RATE_LIMITED', message: 'slow down', status: 429 } }],
      ['llm/retry', { attempt: 1, code: 'RATE_LIMITED', delay_ms: 50 }],
      answer('Done again.', '', spent(200, 20)), end(2, 'error', 'it broke'),
      begin(3), user('Three.'),
    );
    const trace = traced(events);
    // What a turn read and wrote: also the requests that failed, and those on the side.
    expect(trace.turns.map(each => [each.number, each.steps, each.end?.reason ?? null, each.tokens, each.row])).toEqual([
      [1, 1, 'completed', 166, 'event-1'],
      [2, 1, 'error', 253, 'event-7'],
      // A turn that has begun and has no step yet: what was said is in it, and no step reads it yet.
      [3, 0, null, 0, 'event-14'],
    ]);
    // The request that failed is counted, and the one that was answered took from when it was made again.
    expect(trace.steps[1]).toMatchObject({ failed: 1, answer: { tookMs: 50 } });
    expect(trace.steps.map(each => [each.turn, each.step])).toEqual([[1, 1], [2, 1]]);
    expect(trace.rows.at(-1)).toMatchObject({ kind: 'input', step: null, turn: 3 });
    expect(kinds(trace)).toEqual([
      // How a turn ended is told in its last step.
      'input@0', 'reply@0', 'input@1', 'notice@1', 'notice@1', 'reply@1', 'notice@1', 'input@null',
    ]);
  });

  it('says how each call stands', () => {
    const events = log(
      begin(1), user('Go.'), step(1, 1), answer('', ''),
      call('done', 'read', {}), result('done', 'text'),
      call('failed', 'run', {}), ['tool/result', { message: { role: 'tool', call_id: 'failed', content: [{ type: 'text', text: 'no' }] }, error: { code: 'EXIT', reason: 'exit 1' } }],
      call('refused', 'write', {}), ['approval/asked', { call_id: 'refused' }], ['approval/decided', { call_id: 'refused', outcome: 'rejected' }],
      ['tool/result', { message: { role: 'tool', call_id: 'refused', content: [] }, error: { code: 'REJECTED', reason: 'no' } }],
      call('waiting', 'write', {}), ['approval/asked', { call_id: 'waiting' }],
      call('running', 'grep', {}),
    );
    expect(traced(events).steps[0].calls).toEqual(['done', 'failed', 'refused', 'waiting', 'running']);
    // A turn that ends cuts short what has no result.
    expect(traced([...events, { seq: 99, type: 'turn/end', data: { turn: 1, reason: 'aborted' }, time: 9_999 }]).steps[0].calls)
      .toEqual(['done', 'failed', 'refused', 'cut', 'cut']);
  });

  it('marks an answer that was cut short, and gives no step to what came outside of one', () => {
    const events = log(
      user('Before any turn.'),
      begin(1), step(1, 1), answer('Half', '', spent(10, 1), null), end(1, 'aborted'),
    );
    const trace = traced(events);
    expect(trace.rows.find(row => row.kind === 'reply')).toMatchObject({ cut: true, step: 0 });
    expect(trace.rows[0]).toMatchObject({ kind: 'input', turn: null, step: null });
  });

  it('takes no step of a turn other than the one that is under way, nor one outside of every turn', () => {
    const trace = traced(log(
      step(1, 1), begin(2), step(1, 1), step(2, 4), answer('Done.'), end(2),
    ));
    // The step is the fourth of its turn, as the log numbers it.
    expect(trace.steps.map(each => [each.turn, each.step])).toEqual([[2, 4]]);
  });

  it('is empty for a log that holds nothing', () => {
    expect(traced([])).toEqual({ turns: [], steps: [], rows: [] });
  });
});

describe('looking through the rows', () => {
  const events = log(
    begin(1), user('Find the BUG.'), ['step/start', { turn: 1, step: 1 }],
    ['assistant/message', { message: { role: 'assistant', content: [{ type: 'text', text: 'Searching.' }] }, usage: null, finish_reason: 'tool-calls' }],
    call('c1', 'grep', { pattern: 'parse' }),
    ['tool/result', { message: { role: 'tool', call_id: 'c1', content: [{ type: 'text', text: 'src/parser.py:3' }] }, error: { code: 'TIMED_OUT', reason: 'too slow' } }],
    end(1, 'error', 'gave up'),
  );
  const { rows } = traceOf(events, projectEvents(events));
  const tool = rows.find(row => row.kind === 'tool')!;

  it('finds what is sought in what was said, in the name, the arguments, the result and the error of a call', () => {
    expect(rows.filter(row => holds(row, 'bug')).map(row => row.kind)).toEqual(['input']);
    for (const sought of ['GREP', '"pattern"', 'parser.py', 'timed_out', 'too slow']) expect(holds(tool, sought), sought).toBe(true);
    expect(holds(tool, 'nowhere')).toBe(false);
    // Nothing sought is found everywhere.
    expect(rows.every(row => holds(row, '  '))).toBe(true);
    expect(wordsOf(rows[0])).toBe('Find the BUG.');
  });

  it('tells what went wrong: a call that failed or was refused, and a notice that warns', () => {
    expect(rows.filter(wentWrong).map(row => row.kind)).toEqual(['tool', 'notice']);
    expect(wentWrong({ ...tool, state: 'refused' } as TraceRow)).toBe(true);
    expect(wentWrong({ ...tool, state: 'cut' } as TraceRow)).toBe(false);
  });
});

describe('a long task', () => {
  it('is traced at once, however many steps it took', () => {
    // A thousand steps in a hundred turns, each step with an answer, a call and its result.
    const written: Written[] = [];
    for (let turn = 1; turn <= 100; turn++) {
      written.push(begin(turn), user(`Task ${turn}.`));
      for (let step = 1; step <= 10; step++) {
        written.push(
          ['step/start', { turn, step }],
          ['assistant/message', {
            message: { role: 'assistant', content: [{ type: 'reasoning', text: 'A thought.' }, { type: 'text', text: 'An answer.' }] },
            usage: { input_tokens: 1_000, output_tokens: 50, cache_read_tokens: 9_000, cache_write_tokens: 0 },
            finish_reason: 'tool-calls',
          }],
          call(`c-${turn}-${step}`, 'read', { path: 'notes.md' }),
          result(`c-${turn}-${step}`, 'Notes.'),
        );
      }
      written.push(end(turn));
    }
    const events = log(...written);
    const projection = projectEvents(events);
    const began = performance.now();
    const trace = traceOf(events, projection);
    const took = performance.now() - began;
    expect([trace.turns.length, trace.steps.length, trace.rows.length]).toEqual([100, 1_000, 100 + 3 * 1_000]);
    // Far more than it takes, and far less than a trace that looked through every row for each turn would.
    expect(took).toBeLessThan(250);
  });
});
