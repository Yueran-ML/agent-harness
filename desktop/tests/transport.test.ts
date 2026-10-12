import { afterEach, describe, expect, it, vi } from 'vitest';
import { RpcClient, type Transport } from '../src/transport';
import { createMockTransport } from '../src/mock';
import type { RpcMessage, SessionStatus } from '../src/protocol';

function wire() {
  const sent: RpcMessage[] = [];
  let listener: (message: RpcMessage) => void = () => {};
  let closed: (reason: string) => void = () => {};
  const transport: Transport = {
    send: (message) => { sent.push(message); },
    subscribe: (callback) => { listener = callback; return () => {}; },
    onClose: (callback) => { closed = callback; return () => {}; },
    close: () => closed('disconnected'),
  };
  return { client: new RpcClient(transport), sent, receive: (message: RpcMessage) => listener(message), disconnect: () => closed('disconnected') };
}

describe('RpcClient', () => {
  it('matches out-of-order responses by id', async () => {
    const peer = wire();
    const first = peer.client.request('session/list');
    const second = peer.client.request('session/open');
    peer.receive({ jsonrpc: '2.0', id: 2, result: { second: true } });
    peer.receive({ jsonrpc: '2.0', id: 1, result: { first: true } });
    expect(await first).toEqual({ first: true });
    expect(await second).toEqual({ second: true });
  });
  it('keeps the server id namespace separate from pending client requests', async () => {
    const peer = wire();
    const received: RpcMessage[] = [];
    peer.client.subscribe((message) => received.push(message));
    const promise = peer.client.request('session/prompt');
    peer.receive({ jsonrpc: '2.0', id: 1, method: 'approval/ask', params: { call_id: 'same-id' } });
    expect(received).toHaveLength(1);
    peer.client.respond(1, { outcome: 'rejected' });
    peer.receive({ jsonrpc: '2.0', id: 1, result: { accepted: true } });
    expect(await promise).toEqual({ accepted: true });
    expect(peer.sent[1]).toEqual({ jsonrpc: '2.0', id: 1, result: { outcome: 'rejected' } });
  });
  it('rejects outstanding requests and new requests when the transport exits', async () => {
    const peer = wire();
    const waiting = peer.client.request('session/open');
    const check = expect(waiting).rejects.toThrow('disconnected');
    peer.disconnect();
    await check;
    await expect(peer.client.request('session/list')).rejects.toThrow('closed');
  });
});

afterEach(() => vi.useRealTimers());

async function mockSession() {
  const transport = createMockTransport({ delayMs: 1 });
  const client = new RpcClient(transport);
  const messages: RpcMessage[] = [];
  client.subscribe((message) => messages.push(message));
  await client.request('initialize', { protocol: 1, client: { name: 'test', version: '1' } });
  const opened = await client.request<{ session_id: string; events: number }>('session/open');
  return { client, messages, opened };
}
const eventsOf = (messages: RpcMessage[]) => messages.flatMap((message) => 'method' in message && message.method === 'session.event' ? [(message.params as any).event] : []);

describe('replay protocol peer', () => {
  it('matches the three Python permission presets and leaves unknown aliases unchanged', async () => {
    const { client, messages } = await mockSession();
    const change = (line: string) => client.request<{ text: string }>('session/command', { line });
    const statuses = () => messages.flatMap(message =>
      'method' in message && message.method === 'session.status' ? [message.params as unknown as SessionStatus] : []);
    try {
      for (const [preset, mode, approval] of [
        ['full-access', 'danger-full-access', 'never'],
        ['workspace-write', 'workspace-write', 'ask'],
        ['read-only', 'read-only', 'ask'],
      ]) {
        await change(`/PERMISSIONS ${preset}`);
        expect(statuses().at(-1)).toMatchObject({ sandbox: { mode }, approval });
        expect(eventsOf(messages).slice(-2).map(event => event.data)).toEqual([{ mode }, { policy: approval }]);
      }
      const statusCount = statuses().length;
      const eventCount = eventsOf(messages).length;
      for (const alias of ['danger-full-access', 'never', 'ask', 'constructor', 'toString', 'FULL-ACCESS']) {
        expect((await change(`/permissions ${alias}`)).text).toBe('Available presets: read-only, workspace-write, full-access.');
      }
      await change('/permissions\tfull-access');
      expect(statuses()).toHaveLength(statusCount);
      expect(eventsOf(messages)).toHaveLength(eventCount);
      expect((await change('/permissions')).text).toBe('Demo permissions: read-only; approval policy: ask.');
    } finally { client.close(); }
  });

  it('starts empty, lists real history, and preserves the open history/live boundary', async () => {
    const { client, messages, opened } = await mockSession();
    expect(opened.events).toBe(0);
    const list = await client.request<{ sessions: { id: string }[] }>('session/list');
    expect(list.sessions.some((session) => session.id === 'recorded-deepseek-fix')).toBe(true);
    const resumed = await client.request<{ events: number }>('session/open', { session_id: 'recorded-deepseek-fix' });
    expect(resumed.events).toBe(31);
    const first = await client.request<{ events: { seq: number }[]; next: number }>('session/events', { from: 0, limit: 10 });
    expect(first.events.map((event) => event.seq)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(first.next).toBe(10);
    expect(eventsOf(messages)).toHaveLength(0);
    await expect(client.request('session/permissions-unknown')).rejects.toMatchObject({ code: -32601 });
    await client.request('session/prompt', { text: '继续演示' });
    expect(eventsOf(messages)[0].seq).toBe(resumed.events);
    await client.request('session/cancel');
    expect(eventsOf(messages).filter((event) => event.type === 'agent/inbox/spliced').at(-1).data.removed).toBe(1);
    client.close();
  });
  it('finishes a full-access replay without synthesizing an approval request or denial', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const { client, messages } = await mockSession();
    try {
      await client.request('session/command', { line: '/permissions full-access' });
      await client.request('session/prompt', { text: 'Run the full-access replay' });
      await vi.runAllTimersAsync();
      const events = eventsOf(messages);
      expect(events.at(-1)).toMatchObject({ type: 'turn/end', data: { reason: 'completed' } });
      expect(events.some(event => event.type === 'tool/result')).toBe(true);
      expect(events.some(event => event.type === 'approval/asked' || event.type === 'approval/decided')).toBe(false);
      expect(messages.some(message => 'method' in message && message.method === 'approval/ask')).toBe(false);
    } finally { client.close(); }
  });
  it('waits for explicit approval and withdraws it when cancelled, clearing all input queues', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const { client, messages } = await mockSession();
    await client.request('session/prompt', { text: '展示一次修复' });
    await vi.runAllTimersAsync();
    const request = messages.find((message) => 'method' in message && message.method === 'approval/ask');
    expect(request).toBeDefined();
    expect(eventsOf(messages).some((event) => event.type === 'turn/end')).toBe(false);
    await client.request('session/steer', { text: '补充检查' });
    await client.request('session/prompt', { text: '另一个回合' });
    await client.request('session/cancel');
    const events = eventsOf(messages);
    expect(events.some((event) => event.type === 'approval/decided' && event.data.outcome === 'cancelled')).toBe(true);
    expect(events.filter((event) => event.type === 'agent/inbox/spliced' && event.data.removed > 0).map((event) => event.data.queue)).toEqual(expect.arrayContaining(['next-step', 'next-turn']));
    expect(events.at(-1).data.reason).toBe('aborted');
    if (request && 'id' in request && request.id !== null) client.respond(request.id, { outcome: 'allowed-once' });
    await vi.runAllTimersAsync();
    expect(eventsOf(messages).filter((event) => event.type === 'approval/decided')).toHaveLength(1);
    client.close();
  });
  it('consumes steering at the next step and completes after approval', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const { client, messages } = await mockSession();
    client.subscribe((message) => {
      if ('method' in message && message.method === 'approval/ask' && 'id' in message) client.respond(message.id, { outcome: 'allowed-once' });
    });
    await client.request('session/prompt', { text: 'test' });
    await client.request('session/steer', { text: '确认除零边界' });
    await vi.runAllTimersAsync();
    const events = eventsOf(messages);
    expect(events.some((event) => event.type === 'user/message' && event.data.source === 'steer')).toBe(true);
    expect(events.at(-1).data.reason).toBe('completed');
    expect(events.map((event) => event.seq)).toEqual(events.map((_, index) => index));
    client.close();
  });
});
