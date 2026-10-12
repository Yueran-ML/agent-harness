import type {
  ContentBlock, InboxEntry, JsonRecord, Message, SessionEvent, StreamChunk,
  ToolDeclaration, Usage,
} from './protocol';

interface ItemBase {
  id: string;
  seq: number;
  time: number;
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

export interface ToolItem extends ItemBase {
  type: 'tool';
  callId: string;
  name: string;
  arguments: string;
  result: string | null;
  error: { code: string; reason: string } | null;
  durationMs: number | null;
  trimmed: boolean;
  interrupted: boolean;
}

export interface NoticeItem extends ItemBase {
  type: 'notice';
  text: string;
  tone: 'info' | 'warning' | 'error';
}

export interface CompactionItem extends ItemBase {
  type: 'compaction';
  text: string;
}

export type ConversationItem = UserItem | AssistantItem | ToolItem | NoticeItem | CompactionItem;

export interface Projection {
  items: ConversationItem[];
  queues: { 'next-turn': InboxEntry[]; 'next-step': InboxEntry[] };
  usage: Usage;
  lastInputTokens: number;
  tools: ToolDeclaration[];
  compacting: boolean;
  turn: number | null;
  step: number | null;
}

export const emptyUsage = (): Usage => ({
  input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0,
});

export const emptyProjection = (): Projection => ({
  items: [],
  queues: { 'next-turn': [], 'next-step': [] },
  usage: emptyUsage(),
  lastInputTokens: 0,
  tools: [],
  compacting: false,
  turn: null,
  step: null,
});

const record = (value: unknown): JsonRecord =>
  value !== null && typeof value === 'object' && !Array.isArray(value) ? value as JsonRecord : {};
const string = (value: unknown): string => typeof value === 'string' ? value : '';
const number = (value: unknown): number => typeof value === 'number' && Number.isFinite(value) ? value : 0;

function messageOf(value: unknown): Message {
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

function usageOf(value: unknown): Usage {
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

/** Rebuild presentation from the append-only log; model surface replacements do not erase history. */
export function projectEvents(events: readonly SessionEvent[]): Projection {
  const projection = emptyProjection();
  const calls: ToolItem[] = [];
  const resultsBySeq = new Map<number, ToolItem>();
  const approvedAt = new Map<ToolItem, number>();
  const pendingCall = (callId: string) => {
    for (let i = calls.length - 1; i >= 0; i--) {
      if (calls[i].callId === callId && calls[i].result === null && !calls[i].interrupted) return calls[i];
    }
    return undefined;
  };

  for (const event of events) {
    const data = event.data;
    const base = { id: `event-${event.seq}`, seq: event.seq, time: event.time };
    const notice = (text: string, tone: NoticeItem['tone'] = 'info') => {
      projection.items.push({ ...base, type: 'notice', text, tone });
    };

    switch (event.type) {
      case 'user/message': {
        const source = string(data.source);
        const text = messageText(messageOf(data.message));
        if (source === 'compaction') {
          projection.items.push({ ...base, type: 'compaction', text });
        } else if (!source.startsWith('context/') && !isReplacement(event)) {
          projection.items.push({ ...base, type: 'user', source, text });
        }
        break;
      }
      case 'assistant/message': {
        const message = messageOf(data.message);
        const text = messageText(message);
        const reasoning = messageText(message, 'reasoning');
        if (text || reasoning) {
          projection.items.push({
            ...base, type: 'assistant', text, reasoning,
            interrupted: data.finish_reason === null,
          });
        }
        break;
      }
      case 'assistant/attempt': {
        const failure = record(data.failure);
        const detail = string(failure.message) || string(failure.code);
        notice(detail ? `Model request failed: ${detail}` : 'Model request failed', 'warning');
        break;
      }
      case 'tool/call': {
        const item: ToolItem = {
          ...base, type: 'tool', callId: string(data.call_id), name: string(data.name),
          arguments: string(data.arguments), result: null, error: null, durationMs: null, trimmed: false, interrupted: false,
        };
        calls.push(item);
        projection.items.push(item);
        break;
      }
      case 'approval/decided': {
        const call = pendingCall(string(data.call_id));
        if (call) approvedAt.set(call, event.time);
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
            error: null, durationMs: null, trimmed: false, interrupted: false,
          };
          projection.items.push(call);
        } else {
          call.durationMs = Math.max(0, event.time - (approvedAt.get(call) ?? call.time));
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
        projection.queues[queue] = projection.queues[queue].slice(removed).concat(added);
        break;
      }
      case 'request/header': {
        projection.tools = Array.isArray(data.tools) ? data.tools.map(value => {
          const tool = record(value);
          return { name: string(tool.name), description: string(tool.description), parameters: record(tool.parameters) };
        }) : [];
        break;
      }
      case 'turn/start':
        projection.turn = number(data.turn);
        projection.step = null;
        break;
      case 'step/start':
        projection.step = number(data.step);
        break;
      case 'step/end':
        if (projection.step === data.step) projection.step = null;
        break;
      case 'turn/end': {
        for (const call of calls) {
          if (call.result === null) call.interrupted = true;
        }
        if (projection.turn === data.turn) {
          projection.turn = null;
          projection.step = null;
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
          notice(`${endings[reason]}${detail ? `: ${detail}` : ''}`, reason === 'error' ? 'error' : 'info');
        }
        break;
      }
      case 'llm/retry': {
        const code = string(data.code);
        const seconds = number(data.delay_ms) / 1000;
        notice(`Retrying request${code ? ` (${code})` : ''} in ${seconds}s`, 'warning');
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
          notice(`Conversation compaction did not finish${detail ? `: ${detail}` : ''}`, 'warning');
        }
        break;
      }
      case 'session/end-seed':
        if (projection.compacting) {
          projection.compacting = false;
          notice('Previous conversation compaction was interrupted', 'warning');
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
