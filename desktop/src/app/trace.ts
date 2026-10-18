/**
 * What a task did over time, as its log tells it: its turns, the steps of each, what the model
 * was asked and how long it took, the calls of each step, and how fast the model wrote. What
 * somebody said, the calls and the notices are those of the conversation, read from the same
 * projection; each answer of the model is a row of its own, also one that only calls tools.
 * Nothing here draws.
 */
import type { SessionEvent, Usage } from '../protocol';
import { messageOf, messageText, usageOf, type Projection, type ToolItem } from '../state';

/** A number of milliseconds that the log gives, or none where it gives no such number. */
const millisecondsOf = (value: unknown): number => (typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : 0);

/** A request to the model whose answer the log holds. */
interface Answered {
  /** The event of the answer. */
  seq: number;
  /** How long it took, from when it was made to when its answer was in, or null where the log does not tell. */
  tookMs: number | null;
  usage: Usage;
  /** Whether the answer was cut short, so that it tells nothing of how fast the model writes. */
  cut: boolean;
}

/**
 * The requests whose answers the log holds, in their order. A request is made when its step
 * begins, or once what went before it within the step is over: a request that failed, the time
 * to wait before it is made again, or a compaction.
 */
function answeredOf(events: readonly SessionEvent[]): Answered[] {
  const answered: Answered[] = [];
  // When the request that is under way was made, as far as the log tells it.
  let made: number | null = null;
  for (const event of events) {
    const data = event.data;
    switch (event.type) {
      case 'step/start':
        made = event.time;
        break;
      case 'assistant/attempt':
      case 'compaction/end':
        if (made !== null) made = event.time;
        break;
      case 'llm/retry':
        if (made !== null) made = event.time + millisecondsOf(data.delay_ms);
        break;
      case 'assistant/message':
        answered.push({
          seq: event.seq,
          tookMs: made === null ? null : Math.max(0, event.time - made),
          usage: usageOf(data.usage),
          cut: data.finish_reason === null,
        });
        made = null;
        break;
    }
  }
  return answered;
}

/**
 * How many tokens the model wrote a second, over the requests that it answered: what they wrote
 * over the time from when each was made to when its answer was in. That time holds the wait
 * before the model began to write, which the log does not tell apart. An answer that was cut
 * short tells nothing of how fast the model writes, nor does one that wrote nothing. Null where
 * no request tells anything.
 */
export function speedOf(events: readonly SessionEvent[]): number | null {
  let written = 0;
  let spent = 0;
  for (const answer of answeredOf(events)) {
    if (answer.cut || answer.tookMs === null || answer.tookMs <= 0 || answer.usage.output_tokens <= 0) continue;
    written += answer.usage.output_tokens;
    spent += answer.tookMs;
  }
  return spent > 0 ? written / (spent / 1000) : null;
}

/** How a call stands: done, failed, refused before it ran, cut short, waiting for a yes, or running. */
export type CallState = 'done' | 'failed' | 'refused' | 'cut' | 'waiting' | 'running';

/** How a call of the conversation stands. */
export function callStateOf(item: ToolItem): CallState {
  const outcome = item.approval?.outcome;
  if (outcome === 'rejected' || outcome === 'cancelled' || outcome === 'unavailable') return 'refused';
  if (item.interrupted) return 'cut';
  if (item.result === null) return item.approval && outcome === null ? 'waiting' : 'running';
  return item.error ? 'failed' : 'done';
}

/** A step: one request to the model, and the calls of its answer. */
export interface TraceStep {
  turn: number;
  /** Its number within the turn, from 1. */
  step: number;
  seq: number;
  startedAt: number;
  /** The request whose answer was taken, or null while none is in. */
  answer: { tookMs: number | null; usage: Usage; cut: boolean } | null;
  /** How many requests of the step failed before that one, or at all. */
  failed: number;
  /** Whether what somebody said came in for the step to read. */
  input: boolean;
  calls: CallState[];
  /** The first row of the step, or null where it has none yet. */
  row: string | null;
}

interface RowBase {
  id: string;
  seq: number;
  time: number;
  turn: number | null;
  /** The step that the row belongs to, by its place among the steps, or null outside of every step. */
  step: number | null;
}

export type TraceRow =
  | RowBase & { kind: 'input'; text: string; source: string }
  | RowBase & { kind: 'thinking'; text: string }
  | RowBase & { kind: 'reply'; text: string; tookMs: number | null; usage: Usage; cut: boolean }
  | RowBase & { kind: 'tool'; item: ToolItem; state: CallState }
  | RowBase & { kind: 'notice'; text: string; tone: 'info' | 'warning' | 'error' };

export type TraceKind = TraceRow['kind'];

export interface TraceTurn {
  number: number;
  steps: number;
  startedAt: number;
  /** When and how it ended, or null while it runs. */
  end: { time: number; reason: string } | null;
  /** What its requests read and wrote. */
  tokens: number;
  /** The first row of the turn, or null where it has none. */
  row: string | null;
}

export interface Trace {
  turns: TraceTurn[];
  steps: TraceStep[];
  rows: TraceRow[];
}

const total = (usage: Usage) => usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens + usage.output_tokens;

/**
 * The trace of a task: its turns, its steps and its rows, in the order of the log.
 *
 * Args:
 *   events: The log.
 *   projection: What the conversation makes of the same log, which gives the rows.
 */
export function traceOf(events: readonly SessionEvent[], projection: Projection): Trace {
  const answers = new Map(answeredOf(events).map(answer => [answer.seq, answer]));
  const steps: TraceStep[] = [];
  // The step that each event came in, by its place among the steps.
  const stepAt = new Map<number, number>();
  // The turn that each event came in.
  const turnAt = new Map<number, number>();
  // The tokens of each turn.
  const tokens = new Map<number, number>();
  let turn: number | null = null;
  let current: number | null = null;
  for (const event of events) {
    const data = event.data;
    if (event.type === 'turn/start') {
      turn = typeof data.turn === 'number' ? data.turn : null;
      current = null;
    } else if (event.type === 'step/start' && turn !== null && data.turn === turn) {
      steps.push({
        turn, step: typeof data.step === 'number' ? data.step : steps.length + 1, seq: event.seq, startedAt: event.time,
        answer: null, failed: 0, input: false, calls: [], row: null,
      });
      current = steps.length - 1;
    }
    if (turn !== null) turnAt.set(event.seq, turn);
    if (current !== null) {
      stepAt.set(event.seq, current);
      const step = steps[current];
      if (event.type === 'assistant/attempt') step.failed += 1;
      const answer = event.type === 'assistant/message' ? answers.get(event.seq) : undefined;
      if (answer) step.answer = { tookMs: answer.tookMs, usage: answer.usage, cut: answer.cut };
    }
    if (turn !== null && (event.type === 'assistant/message' || event.type === 'assistant/attempt' || event.type === 'request/aside')) {
      tokens.set(turn, (tokens.get(turn) ?? 0) + total(usageOf(data.usage)));
    }
    if (event.type === 'turn/end') {
      turn = null;
      current = null;
    }
  }

  // The first step of each turn, which reads what was said before it began.
  const firstOf = new Map<number, number>();
  steps.forEach((step, index) => { if (!firstOf.has(step.turn)) firstOf.set(step.turn, index); });

  const rows: TraceRow[] = [];
  // Each answer, with what the model thought before it, from its event.
  for (const event of events) {
    const answer = event.type === 'assistant/message' ? answers.get(event.seq) : undefined;
    if (!answer) continue;
    const message = messageOf(event.data.message);
    const base = { seq: event.seq, time: event.time, turn: turnAt.get(event.seq) ?? null, step: stepAt.get(event.seq) ?? null };
    const thought = messageText(message, 'reasoning');
    if (thought) rows.push({ ...base, id: `event-${event.seq}-thinking`, kind: 'thinking', text: thought });
    rows.push({ ...base, id: `event-${event.seq}`, kind: 'reply', text: messageText(message), tookMs: answer.tookMs, usage: answer.usage, cut: answer.cut });
  }
  for (const item of projection.items) {
    const at = stepAt.get(item.seq) ?? null;
    const base = { seq: item.seq, time: item.time, turn: item.turn, step: at };
    switch (item.type) {
      case 'user': {
        // What was said before the first step of its turn is read by that step.
        const step = at ?? (item.turn === null ? null : firstOf.get(item.turn) ?? null);
        rows.push({ ...base, id: item.id, kind: 'input', step, text: item.text, source: item.source });
        if (step !== null) steps[step].input = true;
        break;
      }
      case 'tool':
        rows.push({ ...base, id: item.id, kind: 'tool', item, state: callStateOf(item) });
        break;
      case 'notice':
        rows.push({ ...base, id: item.id, kind: 'notice', text: item.text, tone: item.tone });
        break;
      case 'compaction':
        rows.push({ ...base, id: item.id, kind: 'notice', text: item.text, tone: 'info' });
        break;
    }
  }
  // In the order of the log. What thought before an answer stands before it.
  rows.sort((one, other) => one.seq - other.seq);
  for (const row of rows) {
    if (row.step === null) continue;
    const step = steps[row.step];
    if (step.row === null) step.row = row.id;
    if (row.kind === 'tool') step.calls.push(row.state);
  }

  // The first row of each turn, found in one pass.
  const firstRow = new Map<number, string>();
  for (const row of rows) if (row.turn !== null && !firstRow.has(row.turn)) firstRow.set(row.turn, row.id);
  const turns: TraceTurn[] = projection.turns.map(record => ({
    number: record.number,
    steps: record.steps,
    startedAt: record.startedAt,
    end: record.end ? { time: record.end.time, reason: record.end.reason } : null,
    tokens: tokens.get(record.number) ?? 0,
    row: firstRow.get(record.number) ?? null,
  }));
  return { turns, steps, rows };
}

/** The words of a row that a search looks through. */
export function wordsOf(row: TraceRow): string {
  if (row.kind !== 'tool') return row.text;
  const { item } = row;
  return [item.name, item.arguments, item.result ?? '', item.error ? `${item.error.code} ${item.error.reason}` : ''].join('\n');
}

/** Whether a row holds what is looked for, whatever the case of either. Nothing looked for is found everywhere. */
export function holds(row: TraceRow, sought: string): boolean {
  const wanted = sought.trim().toLowerCase();
  return wanted === '' || wordsOf(row).toLowerCase().includes(wanted);
}

/** Whether a row went wrong: a call that failed, or a notice of something that did. */
export function wentWrong(row: TraceRow): boolean {
  return (row.kind === 'tool' && (row.state === 'failed' || row.state === 'refused'))
    || (row.kind === 'notice' && row.tone !== 'info');
}
