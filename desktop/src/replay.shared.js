import { recording } from './mock-fixture.js';

const copy = (value) => JSON.parse(JSON.stringify(value));
const textMessage = (text) => ({ role: 'user', content: [{ type: 'text', text }] });
const usage = { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0 };
const commands = [
  { name: 'help', summary: 'List the commands provided by this service.' },
  { name: 'model', summary: 'Show or change the current model.' },
  { name: 'permissions', summary: 'Show or change the session permission preset.' },
  { name: 'compact', summary: 'Sum up earlier messages to make room for the next task.' },
];

/** An in-memory protocol peer shared by the browser preview and the stdio process.
 * Model/tool output is a real recorded run; approval and compaction are synthetic.
 * Nothing here executes commands, accesses model providers, or writes a project.
 */
export function createReplayService(output, options = {}) {
  const delayMs = options.delayMs ?? 90;
  const cwd = options.cwd || 'Demo workspace';
  const sessions = new Map();
  const stored = recording.slice(1).map(copy);
  sessions.set('recorded-deepseek-fix', {
    id: 'recorded-deepseek-fix', created_at: recording[0].created_at, events: stored,
    modified_ms: stored.at(-1).time, turn: 1,
  });
  let initialized = false;
  let closed = false;
  let current = null;
  let sessionCounter = 0;
  let requestCounter = 0;
  let generation = 0;
  let running = false;
  let activeTurn = null;
  let compacting = false;
  let approval = null;
  let nextTurn = [];
  let nextStep = [];
  let model = { provider: 'deepseek', model: 'deepseek-flash', context_window: 1048576 };
  let sandbox = { mode: 'workspace-write', strength: 'partial' };
  let policy = 'ask';
  const remembered = new Set();
  const emit = (message) => { if (!closed) output(message); };
  const reply = (id, result) => emit({ jsonrpc: '2.0', id, result });
  const fail = (id, code, message) => emit({ jsonrpc: '2.0', id, error: { code, message } });
  const notify = (method, params) => emit({ jsonrpc: '2.0', method, params });
  const status = () => current && notify('session.status', {
    session_id: current.id, status: running ? 'running' : 'idle', detail: null,
    model: copy(model), sandbox: copy(sandbox), approval: policy,
  });
  const append = (type, data, extra = {}) => {
    const event = { type, seq: current.events.length, time: Date.now(), data: copy(data), ...extra };
    current.events.push(event); current.modified_ms = event.time;
    notify('session.event', { session_id: current.id, event });
    return event;
  };
  const wait = () => new Promise((resolve) => setTimeout(resolve, delayMs));
  const valid = (token) => !closed && token === generation;
  const splice = (queue, removed, added) => append('agent/inbox/spliced', { queue, removed, added });
  const clearQueues = () => {
    if (nextTurn.length) splice('next-turn', nextTurn.length, []);
    if (nextStep.length) splice('next-step', nextStep.length, []);
    nextTurn = []; nextStep = [];
  };
  function cancel() {
    generation++;
    if (!current) return;
    clearQueues();
    if (approval) {
      const pending = approval; approval = null;
      append('approval/decided', { call_id: pending.call_id, outcome: 'cancelled' });
      pending.resolve(false);
    }
    if (activeTurn !== null) append('turn/end', { turn: activeTurn, reason: 'aborted', detail: 'You stopped the replay.' });
    if (compacting) append('compaction/end', { why: 'manual', outcome: { trimmed: 0, summarised: 0, before: 0, after: 0, error: 'You stopped compaction.' } });
    activeTurn = null; compacting = false; running = false; status();
  }
  const consumeSteer = () => {
    if (!nextStep.length) return;
    const entries = nextStep; nextStep = [];
    splice('next-step', entries.length, []);
    for (const entry of entries) append('user/message', { message: entry.message, source: 'steer' }, { surface_op: 'append' });
  };
  async function stream(message, turn, step, token, messageUsage, finishReason) {
    const send = (chunk) => notify('session.stream', { session_id: current.id, turn, step, chunk });
    for (const [index, block] of message.content.entries()) {
      if (!valid(token)) return;
      send({ type: 'block-start', index, kind: block.type, call_id: block.call_id ?? null, name: block.name ?? null });
      const text = block.type === 'tool-call' ? block.arguments : block.text || '';
      const size = block.type === 'tool-call' ? 90 : 54;
      for (let offset = 0; offset < text.length; offset += size) {
        if (!valid(token)) return;
        const piece = text.slice(offset, offset + size);
        send(block.type === 'tool-call' ? { type: 'tool-call-delta', index, arguments: piece }
          : { type: block.type === 'reasoning' ? 'reasoning-delta' : 'text-delta', index, text: piece });
        await wait();
      }
      if (!valid(token)) return;
      send({ type: 'block-end', index, replay_state: null });
    }
    if (valid(token)) {
      send({ type: 'usage', usage: messageUsage || usage });
      send({ type: 'finish', reason: finishReason || 'stop', failure: null });
    }
  }
  async function ask(call, token) {
    const target = JSON.parse(call.arguments).command;
    append('approval/asked', { call_id: call.call_id, tool: call.name, summary: `${call.name}: ${target}` });
    if (policy === 'never' || remembered.has(target)) {
      const allowed = remembered.has(target) && policy !== 'never';
      append('approval/decided', { call_id: call.call_id, outcome: allowed ? 'allowed-once' : 'rejected' });
      return allowed;
    }
    return new Promise((resolve) => {
      const id = ++requestCounter;
      approval = { id, call_id: call.call_id, target, resolve, token };
      emit({ jsonrpc: '2.0', id, method: 'approval/ask', params: {
        session_id: current.id, call_id: call.call_id, tool: call.name,
        summary: `${call.name}: ${target}`,
        reason: 'This approval is a simulation. Allowing it continues the recorded check result without executing the command.',
        target, standing: 'this exact command during this replay service run',
      } });
    });
  }
  async function runQueued() {
    if (!current || running || !nextTurn.length || closed) return;
    running = true; status();
    const token = ++generation;
    const entry = nextTurn[0];
    const turn = ++current.turn;
    await wait();
    if (!valid(token)) return;
    activeTurn = turn;
    append('turn/start', { turn });
    append('sandbox/mode', { mode: sandbox.mode }, { ignorable: true });
    append('approval/policy', { policy }, { ignorable: true });
    nextTurn.shift(); splice('next-turn', 1, []);
    append('user/message', { message: entry.message, source: entry.source }, { surface_op: 'append' });
    const explanation = { role: 'assistant', content: [{ type: 'text', text: 'Demo replay: this recorded run fixes the mean calculation in calc.py and verifies it with check.py. Your input lets you try the interaction; this replay does not execute commands or change files.' }] };
    append('assistant/message', { message: explanation, usage, finish_reason: 'stop' }, { surface_op: 'append' });
    let step = 0;
    for (const original of stored) {
      if (!valid(token)) return;
      if (original.seq < 9 || original.type === 'turn/end') continue;
      const event = copy(original);
      const remap = (id) => `${id}-demo-${turn}`;
      if (event.data.turn !== undefined) event.data.turn = turn;
      if (event.type === 'step/start') { consumeSteer(); step = event.data.step; }
      if (event.data.call_id) event.data.call_id = remap(event.data.call_id);
      if (event.data.message?.call_id) event.data.message.call_id = remap(event.data.message.call_id);
      for (const block of event.data.message?.content || []) if (block.call_id) block.call_id = remap(block.call_id);
      if (event.type === 'assistant/message') {
        await stream(event.data.message, turn, step, token, event.data.usage, event.data.finish_reason);
        if (!valid(token)) return;
      }
      const extra = event.surface_op ? { surface_op: event.surface_op } : {};
      append(event.type, event.data, extra);
      if (event.type === 'tool/call' && event.data.name === 'powershell' && sandbox.mode !== 'danger-full-access') {
        const allowed = await ask(event.data, token);
        if (!valid(token)) return;
        if (!allowed) {
          append('tool/result', { message: { role: 'tool', call_id: event.data.call_id, content: [{ type: 'text', text: 'Approval was denied. The replay has stopped.' }], is_error: true }, error: { code: 'APPROVAL_REJECTED', reason: 'The call was not approved.' }, meta: null }, { surface_op: 'append' });
          append('step/end', { turn, step });
          append('turn/end', { turn, reason: 'blocked', detail: 'The check command was not approved.' });
          activeTurn = null; running = false; status(); void runQueued(); return;
        }
      }
      await wait();
    }
    if (!valid(token)) return;
    // A steer received during the final reply is consumed before finishing the demo turn.
    consumeSteer();
    append('turn/end', { turn, reason: 'completed', detail: null });
    activeTurn = null; running = false;
    if (nextTurn.length) void runQueued(); else status();
  }
  async function command(line, id) {
    if (running) { fail(id, 1003, 'Wait for the current turn to finish before running a command.'); return; }
    // Keep command parsing aligned with Commands.run in the Python service.
    const body = line.trim().slice(1);
    const separator = body.indexOf(' ');
    const name = (separator < 0 ? body : body.slice(0, separator)).toLowerCase();
    const value = separator < 0 ? '' : body.slice(separator + 1).replace(/^[\s\u0085\u001c-\u001f]+|[\s\u0085\u001c-\u001f]+$/g, '');
    if (name === 'help') reply(id, { text: commands.map((item) => `/${item.name} — ${item.summary}`).join('\n') });
    else if (name === 'model') {
      if (value) {
        const split = value.indexOf('/');
        model = { ...model, provider: split > 0 ? value.slice(0, split) : 'deepseek', model: split > 0 ? value.slice(split + 1) : value };
        status();
      }
      reply(id, { text: `Demo model: ${model.provider}/${model.model}. Use /model provider/model to change the displayed model. The recorded output stays the same.` });
    } else if (name === 'permissions') {
      if (value) {
        const presets = {
          'read-only': ['read-only', 'ask'], 'workspace-write': ['workspace-write', 'ask'],
          'full-access': ['danger-full-access', 'never'],
        };
        if (!Object.hasOwn(presets, value)) { reply(id, { text: 'Available presets: read-only, workspace-write, full-access.' }); return; }
        const preset = presets[value];
        sandbox = { ...sandbox, mode: preset[0] }; policy = preset[1];
        append('sandbox/mode', { mode: sandbox.mode }, { ignorable: true });
        append('approval/policy', { policy }, { ignorable: true }); status();
      }
      reply(id, { text: `Demo permissions: ${sandbox.mode}; approval policy: ${policy}.` });
    } else if (name === 'compact') {
      running = true; compacting = true; status();
      const token = ++generation;
      const sources = current.events.filter((event) => event.data.message && event.surface_op === 'append');
      const before = sources.reduce((sum, event) => sum + Math.ceil(JSON.stringify(event.data.message).length / 4), 0);
      append('compaction/start', { why: 'manual', tokens: before });
      await wait(); await wait();
      if (!valid(token)) { reply(id, { text: 'The compaction demo was stopped.' }); return; }
      if (sources.length) {
        const summary = 'Demo summary: the recorded run read calc.py and check.py, then changed the mean denominator from len(values) - 1 to len(values). The original history retains the check result.';
        append('request/aside', { purpose: 'compaction', usage, failure: null });
        const seqs = sources.map((event) => event.seq);
        append('compaction/summary', { summary, replaces: seqs, region_tokens: before, summary_tokens: 64, usage });
        append('user/message', { message: textMessage(summary), source: 'compaction' }, { surface_op: { op: 'replace', start_seq: seqs[0], end_seq: seqs.at(-1) }, source_event_seqs: seqs });
      }
      append('compaction/end', { why: 'manual', outcome: { trimmed: 0, summarised: sources.length, before, after: sources.length ? 64 : 0, error: null } });
      compacting = false; running = false; status(); reply(id, { text: 'The simulated compaction is complete. The full history is still available.' });
      void runQueued();
    } else reply(id, { text: `There is no /${name} command. Use /help to list the available commands.` });
  }
  async function receive(message) {
    if (closed) return;
    if (!message || message.jsonrpc !== '2.0') { fail(message?.id ?? null, -32600, 'This is not a valid JSON-RPC request.'); return; }
    if (!('method' in message)) {
      if (approval && message.id === approval.id) {
        const pending = approval; approval = null;
        const allowed = message.result?.outcome === 'allowed-once' && !message.error;
        if (allowed && message.result.remember === true) remembered.add(pending.target);
        append('approval/decided', { call_id: pending.call_id, outcome: allowed ? 'allowed-once' : 'rejected' });
        pending.resolve(allowed);
      }
      return;
    }
    const { id, method } = message;
    const params = message.params || {};
    if (method === 'initialize') {
      if (initialized) { fail(id, 1000, 'The service has already been initialized.'); return; }
      if (!Number.isInteger(params.protocol) || params.protocol < 1) { fail(id, -32602, 'This protocol version is not supported.'); return; }
      initialized = true;
      reply(id, { protocol: 1, server: { name: 'columbina-replay', version: '0.1.0' }, cwd, commands: copy(commands) }); return;
    }
    if (!initialized) { fail(id, 1000, 'Initialize the service first.'); return; }
    if (method === 'shutdown') { cancel(); reply(id, {}); closed = true; options.onShutdown?.(); return; }
    if (method === 'session/list') {
      reply(id, { sessions: [...sessions.values()].sort((a, b) => b.modified_ms - a.modified_ms).map((session) => {
        const first = session.events.find((event) => event.type === 'user/message' && event.data.source === 'user');
        const text = first?.data.message.content.map((block) => block.text || '').join('').split('\n')[0] || null;
        return { id: session.id, modified_ms: session.modified_ms, first_input: text && text.length > 80 ? `${text.slice(0, 79)}…` : text };
      }) }); return;
    }
    if (method === 'session/open') {
      if (params.session_id !== undefined && (typeof params.session_id !== 'string' || params.session_id.length > 100 || !/^[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?$/.test(params.session_id))) { fail(id, -32602, 'The session identifier is not valid.'); return; }
      cancel();
      const sessionId = params.session_id || `demo-${Date.now()}-${++sessionCounter}`;
      const resumed = sessions.has(sessionId);
      current = sessions.get(sessionId) || { id: sessionId, created_at: Date.now(), modified_ms: Date.now(), events: [], turn: 0 };
      sessions.set(sessionId, current);
      reply(id, { session_id: sessionId, resumed, created_at: current.created_at, events: current.events.length });
      status(); return;
    }
    if (!current) { fail(id, 1001, 'Open a session first.'); return; }
    if (method === 'session/events') {
      if (!Number.isInteger(params.from) || params.from < 0 || (params.limit !== undefined && (!Number.isInteger(params.limit) || params.limit < 1 || params.limit > 500))) { fail(id, -32602, 'The history page parameters are not valid.'); return; }
      const end = Math.min(current.events.length, params.from + (params.limit || 500));
      reply(id, { events: copy(current.events.slice(params.from, end)), next: end < current.events.length ? end : null }); return;
    }
    if (method === 'session/prompt' || method === 'session/steer') {
      const text = typeof params.text === 'string' ? params.text.trim() : '';
      if (!text) { fail(id, -32602, 'The input cannot be empty.'); return; }
      const isSteer = method === 'session/steer' && running && !compacting;
      const entry = { message: textMessage(text), source: isSteer ? 'steer' : 'user' };
      (isSteer ? nextStep : nextTurn).push(entry);
      reply(id, {}); splice(isSteer ? 'next-step' : 'next-turn', 0, [entry]);
      if (!running) void runQueued(); return;
    }
    if (method === 'session/cancel') { reply(id, {}); cancel(); return; }
    if (method === 'session/command') {
      if (typeof params.line !== 'string' || !params.line.trim().startsWith('/')) { fail(id, -32602, 'A command must begin with /.'); return; }
      await command(params.line, id); return;
    }
    fail(id, -32601, `There is no ${method} method.`);
  }
  return { receive, close() { if (!closed) cancel(); closed = true; } };
}
