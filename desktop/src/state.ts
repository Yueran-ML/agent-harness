import type {
  ContentBlock, InboxEntry, JsonRecord, Message, SessionEvent, StreamChunk,
  ToolDeclaration, Usage,
} from './protocol';

interface ItemBase {
  id: string;
  seq: number;
  time: number;
  /** The number of the turn that was under way when the item came, or null outside of every turn. */
  turn: number | null;
}

export interface UserItem extends ItemBase {
  type: 'user';
  text: string;
  source: string;
}

export interface AssistantItem extends ItemBase {
  type: 'assistant';
  text: string;
  reasoning: string;
  interrupted: boolean;
}

/** A question that was put about a call before it ran. */
export interface ToolApproval {
  /** How it ended: `allowed-once`, `rejected`, `cancelled` or `unavailable`. Null while it waits. */
  outcome: string | null;
  /**
   * The policy of approval that the log named when the question was put, or null if it
   * named none. With `never` nobody is asked, and the answer is `rejected` all the same.
   */
  policy: string | null;
}

export interface ToolItem extends ItemBase {
  type: 'tool';
  callId: string;
  name: string;
  arguments: string;
  result: string | null;
  error: { code: string; reason: string } | null;
  /**
   * When the call began to run, in milliseconds since 1970: when it was allowed, where
   * somebody was asked about it, and otherwise when it was made.
   */
  beganAt: number;
  /** How long the call ran, from there to its result. Null while it has none. */
  durationMs: number | null;
  trimmed: boolean;
  interrupted: boolean;
  /** Null for a call that nobody was asked about. */
  approval: ToolApproval | null;
}

/** Why a request to the model failed, as the log has it. */
export interface RequestFailure {
  code: string;
  message: string;
  /** The HTTP status that came with the failure, or null if none did. */
  status: number | null;
  retryAfterMs: number | null;
  requestId: string | null;
}

/**
 * What a notice is about, for an interface that says it in words of its own.
 *
 * Of an `attempt`, `provider` is the service that the request went to, by the
 * name that the log gives it, or null where the log names none. Of a `retry`,
 * `attempt` is the attempt that failed, counted from 1. The next one is made
 * `delayMs` after the notice.
 */
export type NoticeCause =
  | { kind: 'attempt'; failure: RequestFailure; provider: string | null }
  | { kind: 'retry'; attempt: number; code: string; delayMs: number }
  | { kind: 'turn-end'; reason: string; detail: string | null }
  | { kind: 'compaction-failed'; detail: string }
  | { kind: 'compaction-interrupted' }
  | { kind: 'cut-off' };

export interface NoticeItem extends ItemBase {
  type: 'notice';
  text: string;
  tone: 'info' | 'warning' | 'error';
  cause: NoticeCause;
}

export interface CompactionItem extends ItemBase {
  type: 'compaction';
  text: string;
}

export type ConversationItem = UserItem | AssistantItem | ToolItem | NoticeItem | CompactionItem;

/** A turn as the log tells of it: where it stands among the items, and how it ended. */
export interface TurnRecord {
  number: number;
  /** How many items there were when the turn began. Its own items follow from there. */
  at: number;
  seq: number;
  startedAt: number;
  /** How many steps of the turn have begun. A step is one request to the model and the calls it leads to. */
  steps: number;
  /**
   * The service that the requests of the turn went to, by the name that the log gives it:
   * the one that was named last when the turn ended. Null where the log names none.
   */
  provider: string | null;
  /**
   * Null while the turn runs, and for a turn whose end the log does not hold.
   * `code` says for a program what `detail` says in a sentence. An end has none
   * where its reason needs none, and in a log that was written before ends had one.
   */
  end: { seq: number; time: number; reason: string; detail: string | null; code: string | null } | null;
}

/**
 * Input that waited and was dropped unread: a turn was stopped before it read
 * what was said on the way, or a hook kept a task from the model.
 */
export interface DroppedInput {
  /** The event that took it out of its queue. */
  seq: number;
  source: string;
  text: string;
}

export interface Projection {
  items: ConversationItem[];
  turns: TurnRecord[];
  queues: { 'next-turn': InboxEntry[]; 'next-step': InboxEntry[] };
  /** In the order in which it was dropped. */
  dropped: DroppedInput[];
  usage: Usage;
  lastInputTokens: number;
  tools: ToolDeclaration[];
  compacting: boolean;
  turn: number | null;
  step: number | null;
  /**
   * The reply that reached the limit on its length and that the model was told to go on from,
   * by the id of its item, or null. What the model writes next is the same text. Once that is
   * in the log, the two are one item, and this is null again.
   */
  goesOn: string | null;
}

export const emptyUsage = (): Usage => ({
  input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0,
});

export const emptyProjection = (): Projection => ({
  items: [],
  turns: [],
  queues: { 'next-turn': [], 'next-step': [] },
  dropped: [],
  usage: emptyUsage(),
  lastInputTokens: 0,
  tools: [],
  compacting: false,
  turn: null,
  step: null,
  goesOn: null,
});

const record = (value: unknown): JsonRecord =>
  value !== null && typeof value === 'object' && !Array.isArray(value) ? value as JsonRecord : {};
const string = (value: unknown): string => typeof value === 'string' ? value : '';
const number = (value: unknown): number => typeof value === 'number' && Number.isFinite(value) ? value : 0;

export function messageOf(value: unknown): Message {
  const message = record(value);
  return {
    ...message,
    role: string(message.role),
    content: Array.isArray(message.content) ? message.content.map(record) as ContentBlock[] : [],
    call_id: string(message.call_id),
    is_error: message.is_error === true,
  };
}

export function messageText(message: Message, kind = 'text'): string {
  return message.content.filter(block => block.type === kind).map(block => string(block.text)).join('');
}

export function usageOf(value: unknown): Usage {
  const usage = record(value);
  return {
    input_tokens: number(usage.input_tokens),
    output_tokens: number(usage.output_tokens),
    cache_read_tokens: number(usage.cache_read_tokens),
    cache_write_tokens: number(usage.cache_write_tokens),
  };
}

function isReplacement(event: SessionEvent): boolean {
  return typeof event.surface_op === 'object' && event.surface_op?.op === 'replace';
}

function inputTokens(usage: Usage): number {
  return usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens;
}

/**
 * Rebuild presentation from the append-only log; model surface replacements do not erase history.
 *
 * Args:
 *   events: The log, as far as the window has it.
 *   gone: Whether the service that wrote it is gone. A turn that it left under way is then
 *     shown as cut short.
 */
export function projectEvents(events: readonly SessionEvent[], gone = false): Projection {
  const projection = emptyProjection();
  const calls: ToolItem[] = [];
  const resultsBySeq = new Map<number, ToolItem>();
  // The record of the turn that is under way, if one is.
  let openTurn: TurnRecord | null = null;
  // The service that the log named last as the one that requests go to.
  let provider: string | null = null;
  // The policy of approval that the log named last.
  let approvalPolicy: string | null = null;
  // What was taken out of a queue and has not been written down as input since. A turn
  // writes down what it takes. What a stop takes is written down nowhere.
  let taken: DroppedInput[] = [];
  // What the turn that is under way wrote down as input, by the event that holds it. A hook may
  // still keep it from the model: it puts a note in its place.
  const written = new Map<number, DroppedInput>();
  // The reply that the log held last, if it reached the limit on its length: what the model
  // wrote of it, if anything, and the lines that say that the model is to go on. Once such
  // a line is there, the reply that comes next goes on from this one.
  let cutOff: { said: AssistantItem | null; notes: NoticeItem[] } | null = null;
  const settle = () => {
    projection.dropped.push(...taken);
    taken = [];
    written.clear();
  };
  const pendingCall = (callId: string) => {
    for (let i = calls.length - 1; i >= 0; i--) {
      if (calls[i].callId === callId && calls[i].result === null && !calls[i].interrupted) return calls[i];
    }
    return undefined;
  };

  for (const event of events) {
    const data = event.data;
    // An item belongs to the turn that is under way when its event comes. The end of a turn
    // is such an event too, so the line that says how a turn ended belongs to that turn.
    const base = { id: `event-${event.seq}`, seq: event.seq, time: event.time, turn: projection.turn };
    const notice = (text: string, tone: NoticeItem['tone'], cause: NoticeCause) => {
      projection.items.push({ ...base, type: 'notice', text, tone, cause });
    };

    switch (event.type) {
      case 'user/message': {
        const source = string(data.source);
        const text = messageText(messageOf(data.message));
        if (isReplacement(event)) {
          // An event that stands in the place of earlier ones takes nothing out of the queue. Where a
          // hook put it in the place of input of this turn, no model reads that input.
          if (source.startsWith('hook/')) {
            for (const seq of event.source_event_seqs ?? []) {
              const stopped = written.get(seq);
              if (stopped) projection.dropped.push({ ...stopped, seq: event.seq });
            }
          }
        } else {
          const read = taken.findIndex(input => input.source === source && input.text === text);
          if (read >= 0) written.set(event.seq, taken.splice(read, 1)[0]);
        }
        if (source === 'compaction') {
          projection.items.push({ ...base, type: 'compaction', text });
        } else if (source === 'context/cut-off') {
          // The reply before this one reached the limit on its length, and the program had the
          // model go on. What the model says next is the same answer.
          const note: NoticeItem = {
            ...base, type: 'notice', text: 'The reply reached its length limit. The model continues.',
            tone: 'info', cause: { kind: 'cut-off' },
          };
          projection.items.push(note);
          cutOff?.notes.push(note);
        } else if (!source.startsWith('context/') && !isReplacement(event)) {
          projection.items.push({ ...base, type: 'user', source, text });
        }
        // Whatever somebody said stands between: the reply that comes next answers that. What
        // the program tells the model on the way is nothing that anybody said.
        if (!source.startsWith('context/')) cutOff = null;
        break;
      }
      case 'assistant/message': {
        const message = messageOf(data.message);
        const text = messageText(message);
        const reasoning = messageText(message, 'reasoning');
        // A reply goes on from one that was cut off once the model was told to go on. The model
        // wrote on where it had stopped, so the two are one text, and are shown as one answer:
        // it stands where its last part came, and is known by the event that began it.
        const before = cutOff && cutOff.notes.length > 0 ? cutOff : null;
        const began: AssistantItem | null = before?.said ?? null;
        if (before) {
          const gone: ConversationItem[] = [...before.notes, ...(began ? [began] : [])];
          projection.items = projection.items.filter(item => !gone.includes(item));
        }
        let said: AssistantItem | null = null;
        if (began || text || reasoning) {
          const whole: AssistantItem = {
            ...(began ?? base), type: 'assistant',
            text: (began?.text ?? '') + text,
            reasoning: [began?.reasoning, reasoning].filter(Boolean).join('\n\n'),
            interrupted: data.finish_reason === null,
          };
          projection.items.push(whole);
          said = whole;
        }
        cutOff = data.finish_reason === 'max-tokens' ? { said, notes: [] } : null;
        break;
      }
      case 'assistant/attempt': {
        const failure = record(data.failure);
        // No request failed: the reply was cut off before it held anything. The message that
        // tells the model to go on follows, and that is where the interface says so.
        if (string(failure.code) === 'CUT_OFF') break;
        const detail = string(failure.message) || string(failure.code);
        notice(detail ? `Model request failed: ${detail}` : 'Model request failed', 'warning', {
          kind: 'attempt',
          failure: {
            code: string(failure.code),
            message: string(failure.message),
            status: typeof failure.status === 'number' ? failure.status : null,
            retryAfterMs: typeof failure.retry_after_ms === 'number' ? failure.retry_after_ms : null,
            requestId: typeof failure.request_id === 'string' ? failure.request_id : null,
          },
          provider,
        });
        break;
      }
      case 'tool/call': {
        const item: ToolItem = {
          ...base, type: 'tool', callId: string(data.call_id), name: string(data.name),
          arguments: string(data.arguments), result: null, error: null, beganAt: event.time, durationMs: null,
          trimmed: false, interrupted: false, approval: null,
        };
        calls.push(item);
        projection.items.push(item);
        break;
      }
      case 'approval/policy':
        approvalPolicy = string(data.policy) || null;
        break;
      case 'approval/asked': {
        const call = pendingCall(string(data.call_id));
        if (call) call.approval = { outcome: null, policy: approvalPolicy };
        break;
      }
      case 'approval/decided': {
        const call = pendingCall(string(data.call_id));
        if (call) {
          // The time that a person takes to answer is no time of the call.
          call.beganAt = event.time;
          // The policy is the one that held when the question was put, if the log has the question.
          call.approval = {
            outcome: string(data.outcome) || null,
            policy: call.approval ? call.approval.policy : approvalPolicy,
          };
        }
        break;
      }
      case 'tool/result': {
        const message = messageOf(data.message);
        if (isReplacement(event)) {
          // The referenced result is authoritative even if a later call reuses its call_id.
          const source = event.source_event_seqs?.map(seq => resultsBySeq.get(seq)).find(Boolean);
          if (source) {
            source.trimmed = true;
            resultsBySeq.set(event.seq, source);
          }
          break;
        }
        const callId = message.call_id ?? '';
        let call = pendingCall(callId);
        if (!call) {
          call = {
            ...base, type: 'tool', callId, name: 'Tool result', arguments: '', result: null,
            error: null, beganAt: event.time, durationMs: null, trimmed: false, interrupted: false, approval: null,
          };
          projection.items.push(call);
        } else {
          call.durationMs = Math.max(0, event.time - call.beganAt);
        }
        call.result = messageText(message);
        const error = record(data.error);
        call.error = data.error && typeof data.error === 'object'
          ? { code: string(error.code), reason: string(error.reason) }
          : message.is_error ? { code: 'TOOL_ERROR', reason: 'Tool execution failed' } : null;
        resultsBySeq.set(event.seq, call);
        break;
      }
      case 'agent/inbox/spliced': {
        const queue = data.queue;
        if (queue !== 'next-turn' && queue !== 'next-step') break;
        const removed = Math.max(0, Math.trunc(number(data.removed)));
        const added = Array.isArray(data.added) ? data.added.map(value => {
          const entry = record(value);
          return { message: messageOf(entry.message), source: string(entry.source) };
        }) : [];
        for (const entry of projection.queues[queue].slice(0, removed)) {
          taken.push({ seq: event.seq, source: entry.source, text: messageText(entry.message) });
        }
        projection.queues[queue] = projection.queues[queue].slice(removed).concat(added);
        break;
      }
      case 'request/header': {
        projection.tools = Array.isArray(data.tools) ? data.tools.map(value => {
          const tool = record(value);
          return { name: string(tool.name), description: string(tool.description), parameters: record(tool.parameters) };
        }) : [];
        // A header holds for the requests that follow it, until the next one.
        provider = string(record(data.config).provider) || null;
        if (openTurn) openTurn.provider = provider;
        break;
      }
      case 'turn/start':
        // A turn takes what it begins with after it has begun.
        settle();
        projection.turn = number(data.turn);
        projection.step = null;
        openTurn = {
          number: projection.turn, at: projection.items.length, seq: event.seq, startedAt: event.time,
          steps: 0, provider, end: null,
        };
        projection.turns.push(openTurn);
        break;
      case 'step/start':
        projection.step = number(data.step);
        if (openTurn && openTurn.number === data.turn) openTurn.steps += 1;
        break;
      case 'step/end':
        if (projection.step === data.step) projection.step = null;
        break;
      case 'turn/end': {
        settle();
        for (const call of calls) {
          if (call.result === null) call.interrupted = true;
        }
        if (projection.turn === data.turn) {
          projection.turn = null;
          projection.step = null;
          if (openTurn) {
            openTurn.end = {
              seq: event.seq, time: event.time, reason: string(data.reason),
              detail: typeof data.detail === 'string' ? data.detail : null,
              code: typeof data.code === 'string' ? data.code : null,
            };
          }
          openTurn = null;
        }
        const reason = string(data.reason);
        const endings: Record<string, string> = {
          aborted: 'Turn stopped',
          blocked: 'Turn blocked',
          error: 'Turn failed',
          'max-tokens': 'Response reached the output limit',
          interrupted: 'Previous run was interrupted',
          forked: 'Session forked here',
        };
        if (reason !== 'completed' && endings[reason]) {
          const detail = string(data.detail);
          notice(`${endings[reason]}${detail ? `: ${detail}` : ''}`, reason === 'error' ? 'error' : 'info', {
            kind: 'turn-end', reason, detail: typeof data.detail === 'string' ? data.detail : null,
          });
        }
        break;
      }
      case 'llm/retry': {
        const code = string(data.code);
        const seconds = number(data.delay_ms) / 1000;
        notice(`Retrying request${code ? ` (${code})` : ''} in ${seconds}s`, 'warning', {
          kind: 'retry', attempt: number(data.attempt), code, delayMs: number(data.delay_ms),
        });
        break;
      }
      case 'compaction/start':
        projection.compacting = true;
        break;
      case 'compaction/end': {
        projection.compacting = false;
        const outcome = record(data.outcome);
        if (outcome.error) {
          const detail = typeof outcome.error === 'string'
            ? outcome.error : string(record(outcome.error).message);
          notice(`Conversation compaction did not finish${detail ? `: ${detail}` : ''}`, 'warning', {
            kind: 'compaction-failed', detail,
          });
        }
        break;
      }
      case 'session/end-seed':
        if (projection.compacting) {
          projection.compacting = false;
          notice('Previous conversation compaction was interrupted', 'warning', { kind: 'compaction-interrupted' });
        }
        break;
      // System/context updates and unknown event types do not produce conversation cards.
    }

    if (event.type === 'assistant/message' || event.type === 'assistant/attempt' || event.type === 'request/aside') {
      const usage = usageOf(data.usage);
      for (const key of Object.keys(projection.usage) as (keyof Usage)[]) projection.usage[key] += usage[key];
      if (event.type !== 'request/aside') projection.lastInputTokens = inputTokens(usage);
    }
  }
  if (gone && projection.turn !== null) {
    // The service is gone before the turn ended, and nothing more of the turn will come. It
    // is shown as the service writes it down when the task is opened again: cut short, with
    // every call that had no result left without one. What waited in a queue is not given
    // back: the service may still hold it when it is started again.
    for (const call of calls) {
      if (call.result === null) call.interrupted = true;
    }
    const last = events[events.length - 1];
    if (openTurn && openTurn.number === projection.turn) {
      openTurn.end = { seq: last.seq, time: last.time, reason: 'interrupted', detail: null, code: null };
    }
    projection.turn = null;
    projection.step = null;
    projection.compacting = false;
    return projection;
  }
  // While a turn is under way, it may still write down what it took.
  if (projection.turn === null) settle();
  // Nothing is gone on from once the turn is over, whatever the log ended with.
  if (projection.turn !== null && cutOff && cutOff.notes.length > 0) projection.goesOn = cutOff.said?.id ?? null;
  return projection;
}

export interface PreviewBlock {
  index: number;
  kind: string;
  text: string;
  callId: string;
  name: string;
  arguments: string;
}

export interface StreamPreview {
  text: string;
  reasoning: string;
  tools: { index: number; callId: string; name: string; arguments: string }[];
  usage: Usage | null;
  finishReason: string | null;
  blocks: PreviewBlock[];
}

export const emptyPreview = (): StreamPreview => ({
  text: '', reasoning: '', tools: [], usage: null, finishReason: null, blocks: [],
});

/** Preview is ephemeral. The caller clears it when the corresponding durable reply/attempt arrives. */
export function reduceChunk(preview: StreamPreview, chunk: StreamChunk): StreamPreview {
  if (chunk.type === 'usage') return { ...preview, usage: usageOf(chunk.usage) };
  if (chunk.type === 'finish') return { ...preview, finishReason: string(chunk.reason) || null };
  if (!['block-start', 'text-delta', 'reasoning-delta', 'tool-call-delta'].includes(chunk.type)) return preview;
  if (typeof chunk.index !== 'number') return preview;
  const blocks = preview.blocks.map(block => ({ ...block }));
  let block = blocks.find(value => value.index === chunk.index);
  const kind = chunk.type === 'block-start' ? string(chunk.kind)
    : chunk.type === 'text-delta' ? 'text' : chunk.type === 'reasoning-delta' ? 'reasoning' : 'tool-call';
  if (!['text', 'reasoning', 'tool-call'].includes(kind)) return preview;
  if (!block) {
    block = { index: chunk.index, kind, text: '', callId: '', name: '', arguments: '' };
    blocks.push(block);
  }
  if (chunk.type === 'block-start') {
    block.kind = kind;
    block.callId = string(chunk.call_id);
    block.name = string(chunk.name);
  } else if (chunk.type === 'tool-call-delta') {
    block.arguments += string(chunk.arguments);
  } else {
    block.text += string(chunk.text);
  }
  blocks.sort((a, b) => a.index - b.index);
  return {
    ...preview,
    blocks,
    text: blocks.filter(value => value.kind === 'text').map(value => value.text).join(''),
    reasoning: blocks.filter(value => value.kind === 'reasoning').map(value => value.text).join(''),
    tools: blocks.filter(value => value.kind === 'tool-call').map(({ index, callId, name, arguments: args }) => ({
      index, callId, name, arguments: args,
    })),
  };
}
