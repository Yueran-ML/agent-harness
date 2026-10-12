import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Message, SessionEvent, StreamChunk, Usage } from '../src/protocol';
import { emptyPreview, emptyProjection, projectEvents, reduceChunk } from '../src/state';

const message = (text: string, role = 'user'): Message => ({ role, content: [{ type: 'text', text }] });
const usage = (input_tokens = 0, output_tokens = 0, cache_read_tokens = 0, cache_write_tokens = 0): Usage => ({
  input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
});
const event = (seq: number, type: string, data: Record<string, unknown>, time = seq * 100,
  extras: Partial<SessionEvent> = {}): SessionEvent => ({ seq, type, data, time, ...extras });
const result = (call_id: string, text: string) => ({ message: { ...message(text, 'tool'), call_id }, error: null });
const replacement = (seqs: number[]): Partial<SessionEvent> => ({
  surface_op: { op: 'replace', start_seq: seqs[0], end_seq: seqs.at(-1)! }, source_event_seqs: seqs,
});

describe('durable event projection', () => {
  it('keeps original tool output when the model surface is trimmed, including chained replacements', () => {
    const events = [
      event(0, 'tool/call', { call_id: 'read-1', name: 'read', arguments: '{"path":"file.txt"}' }),
      event(1, 'tool/result', result('read-1', 'original full output')),
      event(2, 'tool/result', result('read-1', 'trimmed'), 200, replacement([1])),
      event(3, 'tool/result', result('read-1', 'trimmed again'), 300, replacement([2])),
    ];
    const projection = projectEvents(events);
    expect(projection.items).toHaveLength(1);
    expect(projection.items[0]).toMatchObject({
      type: 'tool', result: 'original full output', trimmed: true, durationMs: 100,
    });
    expect(events[1].data).toEqual(result('read-1', 'original full output'));
    expect(projectEvents(events)).toEqual(projection);
  });

  it('matches repeated call ids to the most recent unpaired call and trims the referenced historical result', () => {
    const projection = projectEvents([
      event(0, 'tool/call', { call_id: 'same', name: 'read', arguments: 'old' }),
      event(1, 'tool/result', result('same', 'old result')),
      event(2, 'tool/call', { call_id: 'same', name: 'read', arguments: 'second' }),
      event(3, 'tool/call', { call_id: 'same', name: 'read', arguments: 'newest' }),
      event(4, 'tool/result', result('same', 'newest result')),
      event(5, 'tool/result', result('same', 'second result')),
      event(6, 'tool/result', result('same', 'trim'), 600, replacement([1])),
    ]);
    expect(projection.items.map(item => item.type === 'tool' && [item.arguments, item.result, item.trimmed])).toEqual([
      ['old', 'old result', true], ['second', 'second result', false], ['newest', 'newest result', false],
    ]);
  });

  it('pairs out-of-order tool results and excludes approval waiting time', () => {
    const projection = projectEvents([
      event(0, 'tool/call', { call_id: 'a', name: 'powershell', arguments: 'run' }, 10),
      event(1, 'tool/call', { call_id: 'b', name: 'read', arguments: 'file' }, 20),
      event(2, 'approval/asked', { call_id: 'a', tool: 'powershell', summary: 'run' }, 30),
      event(3, 'tool/result', result('b', 'read output'), 50),
      event(4, 'approval/decided', { call_id: 'a', outcome: 'allowed-once' }, 30000),
      event(5, 'tool/result', result('a', 'command output'), 30150),
    ]);
    expect(projection.items).toHaveLength(2);
    expect(projection.items[0]).toMatchObject({ callId: 'a', result: 'command output', durationMs: 150 });
    expect(projection.items[1]).toMatchObject({ callId: 'b', result: 'read output', durationMs: 30 });
  });

  it('marks tools without a result as interrupted when a turn ends, without inventing output', () => {
    const projection = projectEvents([
      event(0, 'turn/start', { turn: 1 }),
      event(1, 'tool/call', { call_id: 'done', name: 'read', arguments: '{}' }),
      event(2, 'tool/result', result('done', 'file contents')),
      event(3, 'tool/call', { call_id: 'waiting', name: 'powershell', arguments: '{"command":"test"}' }),
      event(4, 'approval/asked', { call_id: 'waiting', tool: 'powershell', summary: 'test' }),
      event(5, 'approval/decided', { call_id: 'waiting', outcome: 'cancelled' }),
      event(6, 'turn/end', { turn: 1, reason: 'aborted', detail: null }),
    ]);
    const calls = projection.items.filter(item => item.type === 'tool');
    expect(calls[0]).toMatchObject({ result: 'file contents', interrupted: false });
    expect(calls[1]).toMatchObject({ result: null, error: null, durationMs: null, interrupted: true });
    expect(projection.turn).toBeNull();
  });

  it('never attaches a later reused-id result to an interrupted call, including a repaired undispatched call', () => {
    const projection = projectEvents([
      event(0, 'turn/start', { turn: 1 }),
      event(1, 'tool/call', { call_id: 'same', name: 'read', arguments: 'cancelled' }),
      event(2, 'turn/end', { turn: 1, reason: 'aborted' }),
      event(3, 'turn/start', { turn: 2 }),
      event(4, 'tool/call', { call_id: 'same', name: 'read', arguments: 'new call' }),
      event(5, 'tool/result', result('same', 'new output')),
      event(6, 'turn/end', { turn: 2, reason: 'completed' }),
      event(7, 'turn/start', { turn: 3 }),
      event(8, 'assistant/message', { message: { role: 'assistant', content: [
        { type: 'tool-call', call_id: 'same', name: 'read', arguments: 'not dispatched' },
      ] }, usage: usage(), finish_reason: 'tool-calls' }),
      event(9, 'session/end-seed', { inherited: false }),
      event(10, 'tool/result', { ...result('same', 'This call did not run.'), error: { code: 'TOOL_NOT_STARTED', reason: 'The run was interrupted.' } }),
      event(11, 'turn/end', { turn: 3, reason: 'interrupted' }),
    ]);
    const calls = projection.items.filter(item => item.type === 'tool');
    expect(calls).toHaveLength(3);
    expect(calls[0]).toMatchObject({ arguments: 'cancelled', result: null, error: null, interrupted: true });
    expect(calls[1]).toMatchObject({ arguments: 'new call', result: 'new output', interrupted: false });
    expect(calls[2]).toMatchObject({ result: 'This call did not run.', error: { code: 'TOOL_NOT_STARTED' }, interrupted: false });
  });

  it('still pairs recovery results that arrive after the history boundary but before turn end', () => {
    const projection = projectEvents([
      event(0, 'turn/start', { turn: 1 }),
      event(1, 'tool/call', { call_id: 'inflight', name: 'read', arguments: '{}' }),
      event(2, 'session/end-seed', { inherited: false }),
      event(3, 'tool/result', { ...result('inflight', 'Whether it took effect is unknown.'), error: { code: 'TOOL_OUTCOME_UNKNOWN', reason: 'Run interrupted' } }),
      event(4, 'turn/end', { turn: 1, reason: 'interrupted' }),
    ]);
    const calls = projection.items.filter(item => item.type === 'tool');
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ result: 'Whether it took effect is unknown.', error: { code: 'TOOL_OUTCOME_UNKNOWN' }, interrupted: false });
  });

  it('keeps older messages above a single expandable summary and hides context updates', () => {
    const projection = projectEvents([
      event(0, 'user/message', { source: 'user', message: message('first request') }),
      event(1, 'assistant/message', { message: message('first reply', 'assistant'), finish_reason: 'stop' }),
      event(2, 'compaction/start', { why: 'manual' }),
      event(3, 'compaction/summary', { summary: 'summary', usage: usage(300) }),
      event(4, 'user/message', { source: 'compaction', message: message('summary') }, 400, replacement([0, 1])),
      event(5, 'user/message', { source: 'context/runtime', message: message('environment') }),
      event(6, 'user/message', { source: 'context/runtime', message: message('new environment') }, 600, replacement([5])),
      event(7, 'system/message', { message: message('new system', 'system') }),
      event(8, 'compaction/end', { outcome: { trimmed: 0, summarised: 2, error: null } }),
    ]);
    expect(projection.items.map(item => [item.type, 'text' in item && item.text])).toEqual([
      ['user', 'first request'], ['assistant', 'first reply'], ['compaction', 'summary'],
    ]);
    expect(projection.compacting).toBe(false);
    expect(projection.usage.input_tokens).toBe(0);
  });

  it('counts attempts and aside requests once, including both cache fields, with last model context separate', () => {
    const projection = projectEvents([
      event(0, 'assistant/message', { message: message('reply', 'assistant'), usage: usage(10, 2, 30, 4), finish_reason: 'stop' }),
      event(1, 'assistant/attempt', { message: message('partial', 'assistant'), usage: usage(5, 1, 6, 7), failure: { message: 'rate limited', code: 'RATE_LIMIT' } }),
      event(2, 'request/aside', { purpose: 'summary', usage: usage(8, 3, 9, 10) }),
      event(3, 'compaction/summary', { summary: 'summary', usage: usage(8, 3, 9, 10) }),
    ]);
    expect(projection.usage).toEqual(usage(23, 6, 45, 21));
    expect(projection.lastInputTokens).toBe(18);
    expect(projection.items.filter(item => item.type === 'assistant')).toHaveLength(1);
    expect(projection.items[1]).toMatchObject({ type: 'notice', text: 'Model request failed: rate limited' });
  });

  it('applies splices independently to both queues and preserves steer source', () => {
    const entry = (text: string, source = 'user') => ({ message: message(text), source });
    const projection = projectEvents([
      event(0, 'agent/inbox/spliced', { queue: 'next-turn', removed: 0, added: [entry('one'), entry('two')] }),
      event(1, 'agent/inbox/spliced', { queue: 'next-step', removed: 0, added: [entry('change', 'steer')] }),
      event(2, 'agent/inbox/spliced', { queue: 'next-turn', removed: 1, added: [entry('three')] }),
      event(3, 'agent/inbox/spliced', { queue: 'future-queue', removed: 99, added: [entry('unknown')] }),
    ]);
    expect(projection.queues['next-turn'].map(item => item.message.content[0].text)).toEqual(['two', 'three']);
    expect(projection.queues['next-step'][0].source).toBe('steer');
    expect(projectEvents([
      event(0, 'agent/inbox/spliced', { queue: 'next-turn', removed: 0, added: [entry('one')] }),
      event(1, 'agent/inbox/spliced', { queue: 'next-turn', removed: 1, added: [] }),
    ]).queues['next-turn']).toEqual([]);
  });

  describe('input that was dropped unread', () => {
    const entry = (text: string, source = 'user') => ({ message: message(text), source });
    const said = (text: string, source = 'user') => ({ message: message(text), source });

    it('is none where a turn took what waited', () => {
      const projection = projectEvents([
        event(0, 'agent/inbox/spliced', { queue: 'next-turn', removed: 0, added: [entry('fix it')] }),
        event(1, 'turn/start', { turn: 1 }),
        event(2, 'agent/inbox/spliced', { queue: 'next-turn', removed: 1, added: [] }),
        event(3, 'user/message', said('fix it')),
        event(4, 'agent/inbox/spliced', { queue: 'next-step', removed: 0, added: [entry('and test it', 'steer')] }),
        event(5, 'agent/inbox/spliced', { queue: 'next-step', removed: 1, added: [] }),
        event(6, 'user/message', said('and test it', 'steer')),
        event(7, 'turn/end', { turn: 1, reason: 'completed', detail: null }),
      ]);
      expect(projection.dropped).toEqual([]);
    });

    it('is what a stopped turn left unread, with the event that dropped it', () => {
      const projection = projectEvents([
        event(0, 'turn/start', { turn: 1 }),
        event(1, 'agent/inbox/spliced', { queue: 'next-step', removed: 0, added: [entry('never mind', 'steer'), entry('wait', 'steer')] }),
        event(2, 'agent/inbox/spliced', { queue: 'next-step', removed: 2, added: [] }),
        event(3, 'tool/result', result('gone', 'late')),
        event(4, 'turn/end', { turn: 1, reason: 'aborted', detail: 'user' }),
      ]);
      expect(projection.dropped).toEqual([
        { seq: 2, source: 'steer', text: 'never mind' },
        { seq: 2, source: 'steer', text: 'wait' },
      ]);
    });

    it('waits for the log while a turn may still write what it took', () => {
      const events = [
        event(0, 'turn/start', { turn: 1 }),
        event(1, 'agent/inbox/spliced', { queue: 'next-step', removed: 0, added: [entry('one more thing', 'steer')] }),
        event(2, 'agent/inbox/spliced', { queue: 'next-step', removed: 1, added: [] }),
      ];
      expect(projectEvents(events).dropped).toEqual([]);
      expect(projectEvents([...events, event(3, 'user/message', said('one more thing', 'steer'))]).dropped).toEqual([]);
    });

    it('is a task that was dropped before any turn took it', () => {
      const waiting = [
        event(0, 'agent/inbox/spliced', { queue: 'next-turn', removed: 0, added: [entry('first')] }),
        event(1, 'agent/inbox/spliced', { queue: 'next-turn', removed: 1, added: [] }),
      ];
      // No turn is under way, so nothing can still write it down.
      expect(projectEvents(waiting).dropped).toEqual([{ seq: 1, source: 'user', text: 'first' }]);
      // A turn that begins later begins with what was given later.
      expect(projectEvents([
        ...waiting,
        event(2, 'agent/inbox/spliced', { queue: 'next-turn', removed: 0, added: [entry('second')] }),
        event(3, 'turn/start', { turn: 1 }),
        event(4, 'agent/inbox/spliced', { queue: 'next-turn', removed: 1, added: [] }),
        event(5, 'user/message', said('second')),
      ]).dropped).toEqual([{ seq: 1, source: 'user', text: 'first' }]);
    });

    it('is the one of two equal inputs that was not written down', () => {
      const projection = projectEvents([
        event(0, 'turn/start', { turn: 1 }),
        event(1, 'agent/inbox/spliced', { queue: 'next-step', removed: 0, added: [entry('again', 'steer')] }),
        event(2, 'agent/inbox/spliced', { queue: 'next-step', removed: 1, added: [] }),
        event(3, 'user/message', said('again', 'steer')),
        event(4, 'agent/inbox/spliced', { queue: 'next-step', removed: 0, added: [entry('again', 'steer')] }),
        event(5, 'agent/inbox/spliced', { queue: 'next-step', removed: 1, added: [] }),
        event(6, 'turn/end', { turn: 1, reason: 'aborted', detail: 'user' }),
      ]);
      expect(projection.dropped).toEqual([{ seq: 5, source: 'steer', text: 'again' }]);
    });

    it('is only what was taken, and what still waits is not among it', () => {
      const projection = projectEvents([
        event(0, 'agent/inbox/spliced', { queue: 'next-turn', removed: 0, added: [entry('first'), entry('second')] }),
        event(1, 'agent/inbox/spliced', { queue: 'next-turn', removed: 1, added: [] }),
      ]);
      expect(projection.dropped).toEqual([{ seq: 1, source: 'user', text: 'first' }]);
      expect(projection.queues['next-turn'].map(waiting => waiting.message.content[0].text)).toEqual(['second']);
    });

    it('is told from what was written down by its words and by who gave it', () => {
      const taken = [
        event(0, 'turn/start', { turn: 1 }),
        event(1, 'agent/inbox/spliced', { queue: 'next-step', removed: 0, added: [entry('one', 'steer'), entry('two', 'steer')] }),
        event(2, 'agent/inbox/spliced', { queue: 'next-step', removed: 2, added: [] }),
      ];
      const end = event(9, 'turn/end', { turn: 1, reason: 'aborted', detail: 'user' });
      // Only the second was written down.
      expect(projectEvents([...taken, event(3, 'user/message', said('two', 'steer')), end]).dropped)
        .toEqual([{ seq: 2, source: 'steer', text: 'one' }]);
      // The same words from a hook are not the words of the user.
      expect(projectEvents([...taken, event(3, 'user/message', said('one', 'hook')), end]).dropped)
        .toEqual([{ seq: 2, source: 'steer', text: 'one' }, { seq: 2, source: 'steer', text: 'two' }]);
    });

    it('is not taken for written down where an earlier input is written anew', () => {
      const projection = projectEvents([
        event(0, 'turn/start', { turn: 1 }),
        event(1, 'user/message', said('look again', 'steer')),
        event(2, 'agent/inbox/spliced', { queue: 'next-step', removed: 0, added: [entry('look again', 'steer')] }),
        event(3, 'agent/inbox/spliced', { queue: 'next-step', removed: 1, added: [] }),
        // What the model is shown is made shorter: the earlier input is written again, in the place of the old.
        event(4, 'user/message', said('look again', 'steer'), 400, replacement([1])),
        event(5, 'turn/end', { turn: 1, reason: 'aborted', detail: 'user' }),
      ]);
      expect(projection.dropped).toEqual([{ seq: 3, source: 'steer', text: 'look again' }]);
    });

    it('is what a hook kept from the model', () => {
      const projection = projectEvents([
        event(0, 'agent/inbox/spliced', { queue: 'next-turn', removed: 0, added: [entry('secret')] }),
        event(1, 'turn/start', { turn: 1 }),
        event(2, 'agent/inbox/spliced', { queue: 'next-turn', removed: 1, added: [] }),
        event(3, 'turn/end', { turn: 1, reason: 'blocked', detail: 'a hook stopped the input' }),
      ]);
      expect(projection.dropped).toEqual([{ seq: 2, source: 'user', text: 'secret' }]);
    });
  });

  it('finishes orphaned compaction at an inherited-history boundary and reports compaction failure', () => {
    expect(projectEvents([event(0, 'compaction/start', { why: 'pressure' })]).compacting).toBe(true);
    const projection = projectEvents([
      event(0, 'compaction/start', { why: 'pressure' }),
      event(1, 'session/end-seed', { inherited: false }),
      event(2, 'compaction/start', { why: 'manual' }),
      event(3, 'compaction/end', { outcome: { error: '摘要请求失败' } }),
    ]);
    expect(projection.compacting).toBe(false);
    expect(projection.items).toMatchObject([
      { type: 'notice', text: 'Previous conversation compaction was interrupted', tone: 'warning' },
      { type: 'notice', text: 'Conversation compaction did not finish: 摘要请求失败', tone: 'warning' },
    ]);
  });

  it.each([
    ['aborted', 'Turn stopped'], ['blocked', 'Turn blocked'], ['error', 'Turn failed'],
    ['max-tokens', 'Response reached the output limit'], ['interrupted', 'Previous run was interrupted'], ['forked', 'Session forked here'],
  ])('shows the correct explanation for a %s turn', (reason, expected) => {
    const projection = projectEvents([
      event(0, 'turn/start', { turn: 1 }),
      event(1, 'step/start', { turn: 1, step: 2 }),
      event(2, 'turn/end', { turn: 1, reason, detail: 'detail' }),
    ]);
    expect(projection.turn).toBeNull();
    expect(projection.step).toBeNull();
    expect(projection.items[0]).toMatchObject({ type: 'notice', text: `${expected}: detail` });
  });

  it('skips future events and unknown content, keeps text literal, and uses the latest tool declarations', () => {
    const tools = [{ name: 'read', description: 'Read a file', parameters: { type: 'object' } }];
    const projection = projectEvents([
      event(0, 'future/event', { unknown: true }),
      event(1, 'request/header', { tools }),
      event(2, 'request/header', { tools: [{ ...tools[0], name: 'write' }] }),
      event(3, 'assistant/message', { message: { role: 'assistant', content: [
        { type: 'future-block', text: 'hidden' }, { type: 'text', text: '<script>literal</script>' },
      ] }, finish_reason: null }),
    ]);
    expect(projection.items).toMatchObject([{ type: 'assistant', text: '<script>literal</script>', interrupted: true }]);
    expect(projection.tools[0].name).toBe('write');
    expect(emptyProjection()).toEqual(emptyProjection());
  });

  it.each(['deepseek-read', 'deepseek-fix'])('replays the real %s recording without duplicate tool cards', recording => {
    const text = readFileSync(new URL(`../../tests/fixtures/recorded/${recording}/session.v1.jsonl`, import.meta.url), 'utf8');
    const events = text.trim().split(/\r?\n/).slice(1).map(line => JSON.parse(line) as SessionEvent);
    const projection = projectEvents(events);
    const tools = projection.items.filter(item => item.type === 'tool');
    expect(tools).toHaveLength(events.filter(item => item.type === 'tool/call').length);
    expect(tools.every(item => item.result !== null)).toBe(true);
    expect(projection.usage.input_tokens).toBeGreaterThan(0);
    expect(projection.usage.output_tokens).toBeGreaterThan(0);
    expect(projection.turn).toBeNull();
    expect(projection.queues['next-turn']).toEqual([]);
    expect(projectEvents(events)).toEqual(projection);
  });
});

describe('stream previews', () => {
  it('assembles interleaved content by block index without mutating previous previews', () => {
    const chunks: StreamChunk[] = [
      { type: 'block-start', index: 0, kind: 'reasoning' },
      { type: 'reasoning-delta', index: 0, text: 'Think ' },
      { type: 'block-start', index: 1, kind: 'text' },
      { type: 'text-delta', index: 1, text: 'Hello' },
      { type: 'reasoning-delta', index: 0, text: 'again' },
      { type: 'block-start', index: 3, kind: 'tool-call', call_id: 'call', name: 'read' },
      { type: 'tool-call-delta', index: 3, arguments: '{"path":' },
      { type: 'tool-call-delta', index: 3, arguments: '"x"}' },
      { type: 'text-delta', index: 2, text: ' world' },
    ];
    const original = emptyPreview();
    const preview = chunks.reduce(reduceChunk, original);
    expect(original).toEqual(emptyPreview());
    expect(preview.text).toBe('Hello world');
    expect(preview.reasoning).toBe('Think again');
    expect(preview.tools).toEqual([{ index: 3, callId: 'call', name: 'read', arguments: '{"path":"x"}' }]);
    const next = reduceChunk(preview, { type: 'text-delta', index: 1, text: '!' });
    expect(preview.text).toBe('Hello world');
    expect(next.text).toBe('Hello! world');
  });

  it('replaces streaming usage, keeps finish status, and ignores extensions and replay state', () => {
    const preview = [
      { type: 'usage', usage: usage(10, 1) },
      { type: 'usage', usage: usage(10, 3) },
      { type: 'finish', reason: 'tool-calls' },
    ].reduce(reduceChunk, emptyPreview());
    expect(preview.usage).toEqual(usage(10, 3));
    expect(preview.finishReason).toBe('tool-calls');
    expect(reduceChunk(preview, { type: 'future-delta', text: 'ignored' })).toBe(preview);
    expect(reduceChunk(preview, { type: 'block-start', index: 2, kind: 'future-kind' })).toBe(preview);
    expect(reduceChunk(preview, { type: 'block-end', index: 0, replay_state: 'private adapter state' })).toBe(preview);
  });
});
