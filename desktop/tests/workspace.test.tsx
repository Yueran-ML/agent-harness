// @vitest-environment jsdom
/**
 * The workspace of the page against a service process that the test plays: what the page
 * asks of the process, what it makes of what it is told, and what it does when two things
 * happen at once.
 */
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { HISTORY_PAGE } from '../src/app/tell';
import type { SessionEvent, SessionSummary, SkillSummary } from '../src/protocol';
import { useWorkspace } from '../src/useWorkspace';
import { begin, call, end, log, reply, service as listedService, statusOf, user, type Written } from './mounted';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

interface Asked { id: number | string; method: string; params: Record<string, unknown> }
type Handler = (params: Record<string, unknown>) => unknown;

/** What a service process answers a request with when it refuses it. */
class Refused extends Error {
  constructor(readonly code: number, message: string) {
    super(message);
  }
}

/** A promise that the test keeps in its hand. */
function held<T>() {
  let release!: (value: T) => void;
  let refuse!: (problem: Error) => void;
  const promise = new Promise<T>((resolve, reject) => { release = resolve; refuse = reject; });
  return { promise, release, refuse };
}

const hello = { protocol: 1, server: { name: 'columbina', version: '0.1.0' }, cwd: 'F:\\work', commands: [] };
/** The folder that the shell names for a conversation in no project. */
const CHAT = 'C:\\Users\\me\\Columbina\\chat';

/** A service process as the page meets it through the shell, played by the test. */
class Service {
  /** The requests of the page, in the order in which they came. */
  readonly asked: Asked[] = [];
  /** What the page answered to what the service asked of it. */
  readonly answered: { id: number | string; result: unknown }[] = [];
  /** The logs of the tasks, by the name of each. */
  readonly logs = new Map<string, SessionEvent[]>();
  /** The task that is opened where none is named. */
  opens = 's-1';
  sessions: SessionSummary[] = [];
  skills: SkillSummary[] = [];
  recent: { path: string; name: string }[] = [];
  listed: unknown = null;
  tail: string[] = [];
  /** The task that is open. */
  private open = '';
  private readonly lines = new Set<(line: string) => void>();
  private readonly closes = new Set<(reason: string) => void>();
  private readonly shutdowns = new Set<() => void>();
  private readonly handlers = new Map<string, Handler>([
    ['initialize', () => hello],
    ['session/open', (params) => {
      this.open = typeof params.session_id === 'string' ? params.session_id : this.opens;
      return { session_id: this.open, events: this.logs.get(this.open)?.length ?? 0 };
    }],
    ['session/events', params => this.page(this.open, params)],
    ['session/list', () => ({ sessions: this.sessions })],
    ['session/skills', () => ({ skills: this.skills })],
    ['session/prompt', () => ({})],
    ['session/steer', () => ({})],
    ['session/cancel', () => ({})],
    ['session/command', () => ({ text: '' })],
    ['shutdown', () => ({})],
  ]);

  readonly bridge = {
    start: vi.fn(async ({ cwd }: { cwd: string }) => ({ mode: 'service' as const, cwd })),
    startChat: vi.fn(async () => ({ mode: 'service' as const, cwd: CHAT })),
    stop: vi.fn(async () => {}),
    send: (line: string) => this.receive(line),
    onLine: (listener: (line: string) => void) => { this.lines.add(listener); return () => { this.lines.delete(listener); }; },
    onClose: (listener: (reason: string) => void) => { this.closes.add(listener); return () => { this.closes.delete(listener); }; },
    onShutdown: (listener: () => void) => { this.shutdowns.add(listener); return () => { this.shutdowns.delete(listener); }; },
    recentProjects: vi.fn(async () => this.recent),
    services: vi.fn(async () => this.listed),
    serviceTail: vi.fn(async () => this.tail),
    setUpService: vi.fn(async (_asked: { service: string | null; language: string }) => ({ changed: false })),
    selectFolder: vi.fn(async (_asked: { language: string }): Promise<string | null> => null),
    openRecent: vi.fn(async (folder: string) => folder),
    showLog: vi.fn(async () => {}),
  };

  /** Answer one kind of request in another way from now on. */
  on(method: string, handler: Handler): void {
    this.handlers.set(method, handler);
  }

  /** The requests of one kind, as what each was given. */
  of(method: string): Record<string, unknown>[] {
    return this.asked.filter(one => one.method === method).map(one => one.params);
  }

  /** A page of the log of a task, as `session/events` gives it. */
  page(id: string, params: Record<string, unknown>): { events: SessionEvent[]; next: number | null } {
    const all = this.logs.get(id) ?? [];
    const from = Number(params.from);
    const to = from + Number(params.limit);
    return { events: all.slice(from, to), next: to < all.length ? to : null };
  }

  /** Tell the page something that it did not ask for. */
  tell(method: string, params: object): void {
    this.say({ jsonrpc: '2.0', method, params });
  }

  /** Ask the page something, under a number of the service's own. */
  ask(id: number | string, method: string, params: object): void {
    this.say({ jsonrpc: '2.0', id, method, params });
  }

  /** End, as a process does that is stopped or falls over. */
  end(reason: string): void {
    for (const listener of [...this.closes]) listener(reason);
  }

  /** The window is being closed. */
  shutDown(): void {
    for (const listener of [...this.shutdowns]) listener();
  }

  /** A line of the page: a request, or an answer to what the service asked. */
  private receive(line: string): void {
    const message = JSON.parse(line) as { id: number | string; method?: string; params?: Record<string, unknown>; result?: unknown };
    if (message.method === undefined) {
      this.answered.push({ id: message.id, result: message.result });
      return;
    }
    const asked = { id: message.id, method: message.method, params: message.params ?? {} };
    this.asked.push(asked);
    const handler = this.handlers.get(asked.method);
    void Promise.resolve()
      .then(() => {
        if (!handler) throw new Refused(-32601, `There is no method ${asked.method}.`);
        return handler(asked.params);
      })
      .then(
        result => this.say({ jsonrpc: '2.0', id: asked.id, result }),
        (problem: unknown) => this.say({
          jsonrpc: '2.0',
          id: asked.id,
          error: { code: problem instanceof Refused ? problem.code : -32000, message: problem instanceof Error ? problem.message : String(problem) },
        }),
      );
  }

  private say(message: unknown): void {
    for (const listener of [...this.lines]) listener(JSON.stringify(message));
  }
}

/** Mount the workspace over a service process, without a folder. */
function mounted(service = new Service()) {
  vi.stubGlobal('columbina', service.bridge);
  const hook = renderHook(() => useWorkspace());
  return { service, now: () => hook.result.current, unmount: hook.unmount };
}

/** Mount the workspace and open a folder in it. */
async function started(service = new Service(), folder = 'F:\\work') {
  const page = mounted(service);
  await act(async () => { await page.now().openFolder(folder); });
  return page;
}

/** Let everything that is on its way arrive. */
const settled = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
/** A log of four events: one turn, asked and answered. */
const said = () => log(begin(1), user('Fix the parser.'), reply('It is fixed.'), end(1));
/** An event that the service tells of, in the place that it has in its log. */
const event = (seq: number, [type, data = {}]: Written): SessionEvent => ({ seq, type, data, time: 5000 + seq });
/** A log of so many events, for a task that was worked on for long. */
const long = (events: number) => Array.from({ length: events }, (_one, seq) => event(seq, ['step/start']));
const telling = (session: string, one: SessionEvent) => ({ session_id: session, event: one });
const asking = (session: string, called: string) => ({
  session_id: session, call_id: called, tool: 'bash', summary: 'Run a command.', reason: '', target: 'npm test', standing: null,
});
const skill = (name: string): SkillSummary => ({ name, description: `What ${name} is for.`, user_invocable: true, model_invocable: true });

describe('a project that is opened', () => {
  it('starts nothing before a folder is chosen, and knows the folders of earlier days', async () => {
    const service = new Service();
    service.recent = [{ path: 'F:\\work', name: 'work' }];
    const { now } = mounted(service);
    await settled();
    expect(service.bridge.start).not.toHaveBeenCalled();
    expect(service.asked).toEqual([]);
    expect([now().hasProject, now().loading, now().connected, now().hello]).toEqual([false, false, false, null]);
    expect(now().recent).toEqual([{ path: 'F:\\work', name: 'work' }]);
    // A new start is asked for where there is nothing to start: nothing is started.
    await act(async () => { await now().reconnect(); });
    expect(service.bridge.start).not.toHaveBeenCalled();
  });

  it('starts the service process in the folder, greets it, opens a task and reads its log', async () => {
    const service = new Service();
    service.logs.set('s-1', said());
    service.sessions = [{ id: 's-1', modified_ms: 5, first_input: 'Fix the parser.' }];
    service.skills = [skill('review')];
    service.listed = { use: 'deepseek', services: [listedService('deepseek', 'DeepSeek')] };
    const { now } = await started(service);
    // Whatever ran before is stopped first.
    expect(service.bridge.start.mock.calls).toEqual([[{ cwd: 'F:\\work' }]]);
    expect(service.bridge.stop.mock.invocationCallOrder[0]).toBeLessThan(service.bridge.start.mock.invocationCallOrder[0]);
    expect(service.asked.map(one => [one.method, one.params])).toEqual([
      ['initialize', { protocol: 1, client: { name: 'columbina-desktop', version: '0.1.0' } }],
      ['session/open', {}],
      ['session/events', { from: 0, limit: 4 }],
      ['session/list', {}],
      ['session/skills', {}],
    ]);
    const page = now();
    expect([page.hasProject, page.loading, page.connected, page.mode, page.folder]).toEqual([true, false, true, 'service', 'F:\\work']);
    expect(page.hello).toEqual(hello);
    expect([page.sessionId, page.openedAt, page.reading, page.error, page.closed]).toEqual(['s-1', 4, null, null, null]);
    expect(page.events).toEqual(said());
    expect(page.projection.turns).toHaveLength(1);
    expect(page.sessions).toEqual(service.sessions);
    expect(page.skills).toEqual(service.skills);
    expect(page.services).toEqual(service.listed);
    // The folder is the first of the folders of earlier days now: the list is read again.
    expect(service.bridge.recentProjects).toHaveBeenCalledTimes(2);
  });

  it('says what went wrong where the shell cannot start the service process', async () => {
    const service = new Service();
    service.bridge.start.mockRejectedValueOnce(new Error('The folder is gone.'));
    const { now } = await started(service);
    expect([now().error, now().loading, now().connected, now().hello]).toEqual(['The folder is gone.', false, false, null]);
    expect(service.asked).toEqual([]);
    // Another try is not kept from beginning.
    await act(async () => { await now().openFolder('F:\\work'); });
    expect([now().error, now().connected]).toEqual([null, true]);
  });

  it('says what went wrong where the service process refuses the greeting', async () => {
    const service = new Service();
    service.on('initialize', () => { throw new Refused(-32602, 'This service speaks another protocol.'); });
    const { now } = await started(service);
    expect([now().error, now().loading, now().connected, now().hello]).toEqual(['This service speaks another protocol.', false, false, null]);
    expect(service.of('session/open')).toEqual([]);
  });

  it('is started once, however often it is asked for while it starts', async () => {
    const service = new Service();
    const { now } = mounted(service);
    await act(async () => {
      const first = now().openFolder('F:\\work');
      const second = now().openFolder('F:\\work');
      await Promise.all([first, second]);
    });
    expect(service.bridge.start).toHaveBeenCalledTimes(1);
    expect(service.of('initialize')).toHaveLength(1);
  });

  it('lets no folder be chosen while it starts', async () => {
    const service = new Service();
    const starting = held<{ mode: 'service'; cwd: string }>();
    service.bridge.start.mockReturnValueOnce(starting.promise);
    const { now } = mounted(service);
    act(() => { void now().openFolder('F:\\work'); });
    await settled();
    let chosen: string | null = 'not asked';
    await act(async () => { chosen = await now().pickFolder('en'); });
    expect(chosen).toBeNull();
    await act(async () => { chosen = await now().pickRecent('D:\\notes'); });
    expect(chosen).toBeNull();
    expect(service.bridge.selectFolder).not.toHaveBeenCalled();
    expect(service.bridge.openRecent).not.toHaveBeenCalled();
    await act(async () => { starting.release({ mode: 'service', cwd: 'F:\\work' }); });
    await settled();
    expect(now().connected).toBe(true);
  });

  it('is started again in the folder as the shell names it, with the task that was open', async () => {
    const service = new Service();
    service.logs.set('s-7', said());
    service.opens = 's-7';
    // The shell may spell the folder in its own way. That is the folder from then on.
    service.bridge.start.mockResolvedValueOnce({ mode: 'service', cwd: 'F:\\Work' });
    const { now } = await started(service);
    await act(async () => { await now().reconnect(); });
    expect(service.bridge.start.mock.calls).toEqual([[{ cwd: 'F:\\work' }], [{ cwd: 'F:\\Work' }]]);
    expect(service.of('session/open')).toEqual([{}, { session_id: 's-7' }]);
    expect([now().sessionId, now().connected, now().events.length]).toEqual(['s-7', true, 4]);
  });

  it('shows nothing of the service process before while another is started', async () => {
    const service = new Service();
    service.logs.set('s-1', said());
    service.sessions = [{ id: 's-1', modified_ms: 5, first_input: 'Fix the parser.' }];
    service.tail = ['Traceback'];
    service.on('session/command', () => ({ text: 'Done.' }));
    const { now } = await started(service);
    await act(async () => { await now().send('/help'); });
    act(() => {
      service.tell('session.status', statusOf({ status: 'running' }));
      service.ask('q-1', 'approval/ask', asking('s-1', 'c-1'));
      service.tell('session.stream', { session_id: 's-1', turn: 2, step: 1, chunk: { type: 'text-delta', index: 0, text: 'It is ' } });
    });
    act(() => { service.end('It ended with the code 3.'); });
    await settled();
    act(() => { now().fail('Something went wrong.'); });
    expect([now().closed, now().lastWords, now().outputs.length, now().sessions.length]).toEqual(['It ended with the code 3.', ['Traceback'], 1, 1]);

    const starting = held<{ mode: 'service'; cwd: string }>();
    service.bridge.start.mockReturnValueOnce(starting.promise);
    act(() => { void now().reconnect(); });
    await settled();
    const page = now();
    expect([page.loading, page.connected, page.closed, page.error]).toEqual([true, false, null, null]);
    expect([page.hello, page.status, page.sessionId, page.approval]).toEqual([null, null, null, null]);
    expect([page.events, page.sessions, page.outputs, page.lastWords, page.preview.text]).toEqual([[], [], [], [], '']);
    expect(page.reading).toBeNull();
    await act(async () => { starting.release({ mode: 'service', cwd: 'F:\\work' }); });
    await settled();
    expect([now().connected, now().loading, now().sessionId]).toEqual([true, false, 's-1']);
  });

  it('can be given up while it starts, and a start that was given up changes nothing afterwards', async () => {
    const service = new Service();
    const starting = held<{ mode: 'service'; cwd: string }>();
    service.bridge.start.mockReturnValueOnce(starting.promise);
    const { now } = mounted(service);
    act(() => { void now().openFolder('F:\\work'); });
    await settled();
    expect([now().hasProject, now().loading, now().folder]).toEqual([true, true, 'F:\\work']);
    const stopsBefore = service.bridge.stop.mock.calls.length;
    await act(async () => { await now().cancelStart(); });
    expect([now().hasProject, now().loading, now().folder, now().connected, now().error]).toEqual([false, false, null, false, null]);
    // The service that may have started is told to stop.
    expect(service.bridge.stop.mock.calls.length).toBe(stopsBefore + 1);
    await act(async () => { starting.release({ mode: 'service', cwd: 'F:\\work' }); });
    await settled();
    expect(service.asked).toEqual([]);
    expect([now().hasProject, now().loading, now().hello, now().mode]).toEqual([false, false, null, 'mock']);
    // The next start is no longer in the way of the one before.
    await act(async () => { await now().openFolder('D:\\notes'); });
    expect([now().connected, now().folder]).toEqual([true, 'D:\\notes']);
  });

  it('says nothing of a start that was given up and failed afterwards', async () => {
    const service = new Service();
    const starting = held<{ mode: 'service'; cwd: string }>();
    service.bridge.start.mockReturnValueOnce(starting.promise);
    const { now } = mounted(service);
    act(() => { void now().openFolder('F:\\work'); });
    await settled();
    await act(async () => { await now().cancelStart(); });
    await act(async () => { starting.refuse(new Error('Too late to matter.')); });
    await settled();
    expect([now().error, now().loading, now().hasProject]).toEqual([null, false, false]);
  });

  it('can be given up while the shell stops what ran before, and starts nothing then', async () => {
    const service = new Service();
    const stopping = held<void>();
    service.bridge.stop.mockReturnValueOnce(stopping.promise);
    const { now } = mounted(service);
    act(() => { void now().openFolder('F:\\work'); });
    await settled();
    await act(async () => { await now().cancelStart(); });
    await act(async () => { stopping.release(); });
    await settled();
    expect(service.bridge.start).not.toHaveBeenCalled();
    expect([now().hasProject, now().loading]).toEqual([false, false]);
  });

  it('can be given up after it was greeted and has not answered', async () => {
    const service = new Service();
    const greeting = held<typeof hello>();
    service.on('initialize', () => greeting.promise);
    const { now } = mounted(service);
    act(() => { void now().openFolder('F:\\work'); });
    await settled();
    expect(service.of('initialize')).toHaveLength(1);
    await act(async () => { await now().cancelStart(); });
    await act(async () => { greeting.release(hello); });
    await settled();
    expect([now().hello, now().connected, now().hasProject]).toEqual([null, false, false]);
    expect(service.of('session/open')).toEqual([]);
  });

  it('can be given up while its task is opened, and that task is not shown then', async () => {
    const service = new Service();
    service.logs.set('s-1', said());
    const opening = held<{ session_id: string; events: number }>();
    service.on('session/open', () => opening.promise);
    const { now } = mounted(service);
    act(() => { void now().openFolder('F:\\work'); });
    await settled();
    expect([now().hello === null, now().connected, now().loading]).toEqual([false, true, true]);
    await act(async () => { await now().cancelStart(); });
    expect([now().hello, now().connected, now().hasProject, now().loading, now().folder]).toEqual([null, false, false, false, null]);
    await act(async () => { opening.release({ session_id: 's-1', events: 4 }); });
    await settled();
    expect([now().sessionId, now().events, now().loading, now().error]).toEqual([null, [], false, null]);
    expect(service.of('session/events')).toEqual([]);
  });

  it('lets no third start begin beside the second after the first was given up', async () => {
    const service = new Service();
    const first = held<{ mode: 'service'; cwd: string }>();
    const second = held<{ mode: 'service'; cwd: string }>();
    service.bridge.start.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const { now } = mounted(service);
    act(() => { void now().openFolder('F:\\work'); });
    await settled();
    await act(async () => { await now().cancelStart(); });
    act(() => { void now().openFolder('D:\\notes'); });
    await settled();
    // The first start comes to its end now. The second is still under way.
    await act(async () => { first.release({ mode: 'service', cwd: 'F:\\work' }); });
    await settled();
    await act(async () => { await now().openFolder('E:\\other'); });
    expect(service.bridge.start.mock.calls).toEqual([[{ cwd: 'F:\\work' }], [{ cwd: 'D:\\notes' }]]);
    await act(async () => { second.release({ mode: 'service', cwd: 'D:\\notes' }); });
    await settled();
    expect([now().connected, now().folder]).toEqual([true, 'D:\\notes']);
  });

  it('says so where the shell could not stop a start that was given up', async () => {
    const service = new Service();
    const starting = held<{ mode: 'service'; cwd: string }>();
    service.bridge.start.mockReturnValueOnce(starting.promise);
    const { now } = mounted(service);
    act(() => { void now().openFolder('F:\\work'); });
    await settled();
    service.bridge.stop.mockRejectedValueOnce(new Error('The process would not end.'));
    await act(async () => { await now().cancelStart(); });
    expect([now().error, now().hasProject]).toEqual(['The process would not end.', false]);
  });

  it('gives up nothing where nothing is starting', async () => {
    const { service, now } = await started();
    const stops = service.bridge.stop.mock.calls.length;
    await act(async () => { await now().cancelStart(); });
    expect(service.bridge.stop.mock.calls.length).toBe(stops);
    expect([now().connected, now().hasProject]).toEqual([true, true]);
  });
});

describe('a chat, which belongs to no project', () => {
  it('is started by the shell in a folder of its own, and is called a chat', async () => {
    const service = new Service();
    const { now } = mounted(service);
    await act(async () => { await now().openChat(); });
    expect(service.bridge.startChat).toHaveBeenCalledTimes(1);
    expect(service.bridge.start).not.toHaveBeenCalled();
    // Whatever ran before is stopped first.
    expect(service.bridge.stop.mock.invocationCallOrder[0]).toBeLessThan(service.bridge.startChat.mock.invocationCallOrder[0]);
    const page = now();
    expect([page.chat, page.hasProject, page.connected, page.folder, page.sessionId]).toEqual([true, true, true, null, 's-1']);
  });

  it('is started again as a chat, and a project that is opened after it is none', async () => {
    const service = new Service();
    const { now } = mounted(service);
    await act(async () => { await now().openChat(); });
    await act(async () => { await now().reconnect(); });
    expect(service.bridge.startChat).toHaveBeenCalledTimes(2);
    expect(service.bridge.start).not.toHaveBeenCalled();
    // Its folder is no project's, and is not told of as one.
    expect([now().chat, now().folder]).toEqual([true, null]);
    await act(async () => { await now().openFolder('F:\\work'); });
    expect(service.bridge.start.mock.calls).toEqual([[{ cwd: 'F:\\work' }]]);
    expect([now().chat, now().folder]).toEqual([false, 'F:\\work']);
    // And the project is started again as the project.
    await act(async () => { await now().reconnect(); });
    expect(service.bridge.start.mock.calls).toEqual([[{ cwd: 'F:\\work' }], [{ cwd: 'F:\\work' }]]);
    expect(service.bridge.startChat).toHaveBeenCalledTimes(2);
    // A chat after a project is a chat again, also when it is started again.
    await act(async () => { await now().openChat(); });
    expect([now().chat, now().folder, service.bridge.startChat.mock.calls.length]).toEqual([true, null, 3]);
    await act(async () => { await now().reconnect(); });
    expect([now().chat, now().folder, service.bridge.startChat.mock.calls.length]).toEqual([true, null, 4]);
    expect(service.bridge.start).toHaveBeenCalledTimes(2);
  });

  it('is not begun while a project starts, and a chat that is given up while it starts is none', async () => {
    const service = new Service();
    const starting = held<{ mode: 'service'; cwd: string }>();
    service.bridge.start.mockImplementationOnce(() => starting.promise);
    const { now } = mounted(service);
    let opened!: Promise<void>;
    act(() => { opened = now().openFolder('F:\\work'); });
    await settled();
    await act(async () => { await now().openChat(); });
    expect(service.bridge.startChat).not.toHaveBeenCalled();
    expect(now().chat).toBe(false);
    starting.release({ mode: 'service', cwd: 'F:\\work' });
    await act(async () => { await opened; });
    expect([now().chat, now().connected]).toEqual([false, true]);

    const chatting = held<{ mode: 'service'; cwd: string }>();
    service.bridge.startChat.mockImplementationOnce(() => chatting.promise);
    let chat!: Promise<void>;
    act(() => { chat = now().openChat(); });
    await settled();
    expect(now().chat).toBe(true);
    await act(async () => { await now().cancelStart(); });
    expect([now().chat, now().hasProject]).toEqual([false, false]);
    chatting.release({ mode: 'service', cwd: CHAT });
    await act(async () => { await chat; });
    expect([now().chat, now().hasProject]).toEqual([false, false]);
    // Nothing is there to start again: not the chat that was given up.
    await act(async () => { await now().reconnect(); });
    expect(service.bridge.startChat).toHaveBeenCalledTimes(1);
  });
});

describe('what the service process tells of', () => {
  it('is put into the log of the open task in its place, once', async () => {
    const service = new Service();
    service.logs.set('s-1', said());
    const { now } = await started(service);
    act(() => {
      service.tell('session.event', telling('s-1', event(5, user('And the tests.'))));
      service.tell('session.event', telling('s-1', event(4, begin(2))));
      service.tell('session.event', telling('s-1', event(5, user('And the tests.'))));
    });
    expect(now().events.map(one => one.seq)).toEqual([0, 1, 2, 3, 4, 5]);
    // What is told of another task is not shown in this one, and neither is what is no event.
    act(() => {
      service.tell('session.event', telling('s-2', event(6, user('Elsewhere.'))));
      service.tell('session.event', { session_id: 's-1' });
      service.tell('session.event', { session_id: 's-1', event: { type: 'user/message', data: {} } });
    });
    expect(now().events.map(one => one.seq)).toEqual([0, 1, 2, 3, 4, 5]);
  });

  it('says how the open task stands, and keeps what it hears of another task for when that is opened', async () => {
    const service = new Service();
    const { now } = await started(service);
    expect(now().status).toBeNull();
    act(() => { service.tell('session.status', statusOf({ status: 'running' })); });
    expect(now().status?.status).toBe('running');
    act(() => { service.tell('session.status', statusOf({ session_id: 's-2', status: 'stopped' })); });
    expect(now().status?.status).toBe('running');
    await act(async () => { await now().openSession('s-2'); });
    expect([now().sessionId, now().status?.status]).toEqual(['s-2', 'stopped']);
  });

  it('shows a reply while it is written, and no longer once it is in the log', async () => {
    const service = new Service();
    service.logs.set('s-1', said());
    const { now } = await started(service);
    const chunk = (step: number, text: string, session = 's-1', turn = 2) =>
      ({ session_id: session, turn, step, chunk: { type: 'text-delta', index: 0, text } });
    act(() => {
      service.tell('session.stream', chunk(1, 'It is '));
      service.tell('session.stream', chunk(1, 'fixed.'));
      service.tell('session.stream', chunk(1, 'Elsewhere.', 's-2'));
    });
    expect(now().preview.text).toBe('It is fixed.');
    // The next step begins with nothing written, and so does the same step of the next turn.
    act(() => { service.tell('session.stream', chunk(2, 'Now ')); });
    expect(now().preview.text).toBe('Now ');
    act(() => { service.tell('session.stream', chunk(2, 'Then ', 's-1', 3)); });
    expect(now().preview.text).toBe('Then ');
    act(() => { service.tell('session.event', telling('s-1', event(4, reply('Now the tests.')))); });
    expect(now().preview.text).toBe('');
    // What is written after that is of a reply of its own, also in the same step.
    act(() => { service.tell('session.stream', chunk(2, 'And ', 's-1', 3)); });
    expect(now().preview.text).toBe('And ');
    // A reply that failed is in the log as an attempt, and the end of a turn ends whatever was written.
    act(() => { service.tell('session.event', telling('s-1', event(5, ['assistant/attempt', { message: { role: 'assistant', content: [] }, failure: null }]))); });
    expect(now().preview.text).toBe('');
    act(() => { service.tell('session.stream', chunk(3, 'Once more ', 's-1', 3)); });
    act(() => { service.tell('session.event', telling('s-1', event(6, end(3)))); });
    expect(now().preview.text).toBe('');
    // Any other event leaves what is being written as it is.
    act(() => { service.tell('session.stream', chunk(1, 'Later ', 's-1', 4)); });
    act(() => { service.tell('session.event', telling('s-1', event(7, call('c-1', 'read', { path: 'a.py' })))); });
    expect(now().preview.text).toBe('Later ');
  });

  it('asks for the list of tasks again when a turn is over', async () => {
    const service = new Service();
    const { now } = await started(service);
    expect(service.of('session/list')).toHaveLength(1);
    service.sessions = [{ id: 's-1', modified_ms: 9, first_input: 'Fix the parser.' }];
    act(() => { service.tell('session.event', telling('s-1', event(0, begin(1)))); });
    expect(service.of('session/list')).toHaveLength(1);
    act(() => { service.tell('session.event', telling('s-1', event(1, end(1)))); });
    await settled();
    expect(service.of('session/list')).toHaveLength(2);
    expect(now().sessions).toEqual(service.sessions);
  });

  it('shows a question of the agent, and answers that question and no other', async () => {
    const service = new Service();
    const { now } = await started(service);
    // What is no request is no question, whatever it is called.
    act(() => { service.tell('approval/ask', asking('s-1', 'c-1')); });
    expect(now().approval).toBeNull();
    act(() => { service.ask('q-1', 'approval/ask', asking('s-1', 'c-1')); });
    expect(now().approval).toEqual({ ...asking('s-1', 'c-1'), requestId: 'q-1' });
    // An answer that was meant for another question is not sent.
    act(() => { now().answer('q-0', 'allowed-once'); });
    expect(service.answered).toEqual([]);
    act(() => { now().answer('q-1', 'allowed-once', true); });
    expect(service.answered).toEqual([{ id: 'q-1', result: { outcome: 'allowed-once', remember: true } }]);
    // The question is gone once the log says that it was decided: this call, in this task.
    expect(now().approval).not.toBeNull();
    act(() => { service.tell('session.event', telling('s-1', event(0, ['approval/decided', { call_id: 'c-0' }]))); });
    expect(now().approval).not.toBeNull();
    act(() => { service.tell('session.event', telling('s-2', event(0, ['approval/decided', { call_id: 'c-1' }]))); });
    expect(now().approval).not.toBeNull();
    act(() => { service.tell('session.event', telling('s-1', event(1, ['approval/decided', { call_id: 'c-1' }]))); });
    expect(now().approval).toBeNull();
    act(() => { now().answer('q-1', 'rejected'); });
    expect(service.answered).toHaveLength(1);
  });

  it('answers a question with a no that holds for this once', async () => {
    const service = new Service();
    const { now } = await started(service);
    act(() => { service.ask(41, 'approval/ask', asking('s-1', 'c-1')); });
    act(() => { now().answer(41, 'rejected'); });
    expect(service.answered).toEqual([{ id: 41, result: { outcome: 'rejected', remember: false } }]);
  });

  it('shows no question that is asked in another task, now or when that task is opened later', async () => {
    const service = new Service();
    service.logs.set('s-2', log(begin(1), user('Write the notes.')));
    const { now } = await started(service);
    act(() => {
      service.ask('q-1', 'approval/ask', asking('s-2', 'c-1'));
      service.tell('session.event', telling('s-2', event(7, reply('Of an earlier time.'))));
    });
    expect(now().approval).toBeNull();
    // Whatever was heard of the other task before it was asked for is of an earlier time. Its
    // log says what is true of it, and a question of that time has nobody left to answer to.
    await act(async () => { await now().openSession('s-2'); });
    expect([now().approval, now().events.map(one => one.seq)]).toEqual([null, [0, 1]]);
  });
});

describe('what the user sends', () => {
  it('is a task for an agent at rest and words for the next step of one at work', async () => {
    const service = new Service();
    const { now } = await started(service);
    let taken = false;
    await act(async () => { taken = await now().send('Fix the parser.'); });
    expect(taken).toBe(true);
    act(() => { service.tell('session.status', statusOf({ status: 'running' })); });
    await act(async () => { taken = await now().send('Mind the tests.'); });
    expect(taken).toBe(true);
    expect(service.asked.slice(-2).map(one => [one.method, one.params])).toEqual([
      ['session/prompt', { text: 'Fix the parser.' }],
      ['session/steer', { text: 'Mind the tests.' }],
    ]);
  });

  it('is read at once by an agent at work where it is to be, and is a task for one at rest all the same', async () => {
    const service = new Service();
    const { now } = await started(service);
    await act(async () => { await now().send('Fix the parser.', false, true); });
    act(() => { service.tell('session.status', statusOf({ status: 'running' })); });
    await act(async () => { await now().send('Mind the tests.', false, true); });
    expect(service.asked.slice(-2).map(one => [one.method, one.params])).toEqual([
      ['session/prompt', { text: 'Fix the parser.' }],
      ['session/steer', { text: 'Mind the tests.', now: true }],
    ]);
  });

  it('is nothing where it is made of blanks', async () => {
    const service = new Service();
    const { now } = await started(service);
    const before = service.asked.length;
    let taken = true;
    await act(async () => { taken = await now().send('   '); });
    expect(taken).toBe(false);
    expect(service.asked).toHaveLength(before);
  });

  it('is said to be on its way until the service process has taken it', async () => {
    const service = new Service();
    const taking = held<object>();
    service.on('session/prompt', () => taking.promise);
    const { now } = await started(service);
    let sent: Promise<boolean> = Promise.resolve(false);
    act(() => { sent = now().send('Fix the parser.'); });
    await settled();
    expect(now().sending).toBe(true);
    await act(async () => { taking.release({}); await sent; });
    expect(now().sending).toBe(false);
    expect(await sent).toBe(true);
  });

  it('is said not to have gone through where the service process refuses it, until something is sent again', async () => {
    const service = new Service();
    service.on('session/prompt', () => { throw new Refused(1002, 'A turn is under way.'); });
    const { now } = await started(service);
    let taken = true;
    await act(async () => { taken = await now().send('Fix the parser.'); });
    expect([taken, now().error, now().sending]).toEqual([false, 'A turn is under way.', false]);
    service.on('session/prompt', () => ({}));
    await act(async () => { taken = await now().send('Fix the parser.'); });
    expect([taken, now().error]).toEqual([true, null]);
    act(() => { now().fail('Something else.'); });
    expect(now().error).toBe('Something else.');
    act(() => { now().clearError(); });
    expect(now().error).toBeNull();
    // What went wrong is said in words, whatever it was that was thrown.
    act(() => { now().fail(404); });
    expect(now().error).toBe('404');
  });

  it('is carried out as a command where it begins with a slash, and what the command answers stands after the log', async () => {
    const service = new Service();
    service.logs.set('s-1', said());
    service.on('session/command', params => ({ text: `Done: ${String(params.line)}` }));
    const { now } = await started(service);
    await act(async () => { await now().send('  /model deepseek-pro '); });
    expect(service.of('session/command')).toEqual([{ line: '/model deepseek-pro' }]);
    expect(service.of('session/prompt')).toEqual([]);
    expect(now().outputs).toEqual([{ id: 1, line: '/model deepseek-pro', text: 'Done: /model deepseek-pro', after: 3 }]);
    act(() => { service.tell('session.event', telling('s-1', event(4, begin(2)))); });
    await act(async () => { await now().send('/permissions'); });
    expect(now().outputs.map(one => [one.id, one.after])).toEqual([[1, 3], [2, 4]]);
  });

  it('leaves out what a command answers where that was asked for, and where it answers nothing', async () => {
    const service = new Service();
    service.on('session/command', params => ({ text: params.line === '/clear' ? '' : 'Done.' }));
    const { now } = await started(service);
    let taken = false;
    await act(async () => { taken = await now().send('/permissions read-only', true); });
    expect(taken).toBe(true);
    await act(async () => { await now().send('/clear'); });
    expect(service.of('session/command')).toEqual([{ line: '/permissions read-only' }, { line: '/clear' }]);
    expect(now().outputs).toEqual([]);
    // In a task with no event yet, what a command answers stands before everything.
    await act(async () => { await now().command('/help'); });
    expect(now().outputs).toEqual([{ id: 1, line: '/help', text: 'Done.', after: -1 }]);
  });

  it('is given to the agent as words, whatever it begins with, where that is what was asked for', async () => {
    const service = new Service();
    const { now } = await started(service);
    await act(async () => { await now().prompt('/what is a mean?'); });
    expect(service.of('session/prompt')).toEqual([{ text: '/what is a mean?' }]);
    expect(service.of('session/command')).toEqual([]);
  });

  it('stops the turn, and says so until the service process says that no turn runs', async () => {
    const service = new Service();
    const { now } = await started(service);
    act(() => { service.tell('session.status', statusOf({ status: 'running' })); });
    await act(async () => { await now().cancel(); });
    expect(service.of('session/cancel')).toHaveLength(1);
    expect(now().cancelling).toBe(true);
    act(() => { service.tell('session.status', statusOf({ status: 'running', detail: 'still at it' })); });
    expect(now().cancelling).toBe(true);
    // What is said of another task says nothing of this one.
    act(() => { service.tell('session.status', statusOf({ session_id: 's-2', status: 'idle' })); });
    expect(now().cancelling).toBe(true);
    act(() => { service.tell('session.status', statusOf({ status: 'idle' })); });
    expect(now().cancelling).toBe(false);
  });

  it('says that the turn could not be stopped where the service process refuses to', async () => {
    const service = new Service();
    service.on('session/cancel', () => { throw new Refused(1003, 'No turn is under way.'); });
    const { now } = await started(service);
    await act(async () => { await now().cancel(); });
    expect([now().cancelling, now().error]).toEqual([false, 'No turn is under way.']);
  });
});

describe('another task that is opened', () => {
  const two = () => {
    const service = new Service();
    service.logs.set('s-1', said());
    service.logs.set('s-2', log(begin(1), user('Write the notes.')));
    return service;
  };

  it('takes the place of the one that was shown, with its own log and nothing of the other', async () => {
    const service = two();
    service.on('session/command', () => ({ text: 'Done.' }));
    const { now } = await started(service);
    await act(async () => { await now().send('/help'); });
    act(() => {
      service.tell('session.status', statusOf({ status: 'running' }));
      service.ask('q-1', 'approval/ask', asking('s-1', 'c-1'));
      service.tell('session.stream', { session_id: 's-1', turn: 2, step: 1, chunk: { type: 'text-delta', index: 0, text: 'It is ' } });
    });
    await act(async () => { await now().cancel(); });
    expect([now().outputs.length, now().approval === null, now().cancelling, now().preview.text]).toEqual([1, false, true, 'It is ']);
    service.skills = [skill('review')];
    await act(async () => { await now().openSession('s-2'); });
    expect(service.of('session/open')).toEqual([{}, { session_id: 's-2' }]);
    expect([now().sessionId, now().openedAt, now().loading]).toEqual(['s-2', 2, false]);
    expect(now().events.map(one => one.type)).toEqual(['turn/start', 'user/message']);
    expect([now().outputs, now().approval, now().cancelling, now().status, now().preview.text]).toEqual([[], null, false, null, '']);
    // The skills are those of the task that is open now, and the list of tasks is read again.
    expect(now().skills).toEqual(service.skills);
    expect(service.asked.slice(-3).map(one => one.method)).toEqual(['session/events', 'session/list', 'session/skills']);
    // What is written in the task after that begins with nothing, whatever was written in the other.
    act(() => { service.tell('session.stream', { session_id: 's-2', turn: 2, step: 1, chunk: { type: 'text-delta', index: 0, text: 'The notes' } }); });
    expect(now().preview.text).toBe('The notes');
  });

  it('keeps what is told of it while it is being opened, and shows it once it is', async () => {
    const service = two();
    const opening = held<{ session_id: string; events: number }>();
    const { now } = await started(service);
    service.on('session/open', () => opening.promise);
    act(() => { void now().openSession('s-2'); });
    await settled();
    expect([now().loading, now().sessionId]).toEqual([true, 's-1']);
    act(() => {
      service.tell('session.event', telling('s-2', event(2, reply('The notes.'))));
      service.tell('session.event', telling('s-2', event(3, end(1))));
      service.tell('session.status', statusOf({ session_id: 's-2', status: 'running' }));
      service.ask('q-9', 'approval/ask', asking('s-2', 'c-9'));
    });
    // The task that is shown is still the first, and nothing of the other is shown in it.
    expect([now().events.length, now().approval, now().status]).toEqual([4, null, null]);
    service.on('session/events', () => ({ events: service.logs.get('s-2'), next: null }));
    await act(async () => { opening.release({ session_id: 's-2', events: 2 }); });
    await settled();
    expect(now().events.map(one => one.seq)).toEqual([0, 1, 2, 3]);
    expect([now().status?.status, now().approval?.requestId]).toEqual(['running', 'q-9']);
  });

  it('shows no question that was decided while the task was being opened', async () => {
    const service = two();
    const opening = held<{ session_id: string; events: number }>();
    const { now } = await started(service);
    service.on('session/open', () => opening.promise);
    act(() => { void now().openSession('s-2'); });
    await settled();
    act(() => {
      service.ask('q-9', 'approval/ask', asking('s-2', 'c-9'));
      service.tell('session.event', telling('s-2', event(2, ['approval/decided', { call_id: 'c-9' }])));
    });
    await act(async () => { opening.release({ session_id: 's-2', events: 2 }); });
    await settled();
    expect([now().sessionId, now().approval]).toEqual(['s-2', null]);
  });

  it('leaves the task that was shown as it is where the other cannot be opened', async () => {
    const service = two();
    const { now } = await started(service);
    service.on('session/open', () => { throw new Refused(1001, 'There is no such session.'); });
    await act(async () => { await now().openSession('s-9'); });
    expect([now().sessionId, now().events.length, now().loading, now().error]).toEqual(['s-1', 4, false, 'There is no such session.']);
    // An event of the first task is still an event of the task that is shown.
    act(() => { service.tell('session.event', telling('s-1', event(4, begin(2)))); });
    expect(now().events).toHaveLength(5);
    // What went wrong is no longer said once another task is opened.
    service.on('session/open', params => ({ session_id: String(params.session_id), events: 2 }));
    service.on('session/events', params => service.page('s-2', params));
    await act(async () => { await now().openSession('s-2'); });
    expect([now().sessionId, now().error]).toEqual(['s-2', null]);
  });

  it('is asked for once while the service process has not answered', async () => {
    const service = two();
    const opening = held<{ session_id: string; events: number }>();
    const { now } = await started(service);
    service.on('session/open', () => opening.promise);
    act(() => { void now().openSession('s-2'); });
    await settled();
    await act(async () => { await now().openSession('s-1'); });
    expect(service.of('session/open')).toEqual([{}, { session_id: 's-2' }]);
    await act(async () => { opening.release({ session_id: 's-2', events: 2 }); });
    await settled();
    expect(now().sessionId).toBe('s-2');
  });

  it('counts the events of a long log as they are read, a page at a time', async () => {
    const service = new Service();
    const total = HISTORY_PAGE * 2 + 10;
    service.logs.set('s-long', long(total));
    const first = held<void>();
    const second = held<void>();
    const { now } = await started(service);
    const pages = service.asked.length;
    service.on('session/events', async (params) => {
      await (params.from === 0 ? first : params.from === HISTORY_PAGE ? second : { promise: undefined }).promise;
      return service.page('s-long', params);
    });
    act(() => { void now().openSession('s-long'); });
    await settled();
    expect([now().loading, now().reading, now().events.length]).toEqual([true, { read: 0, of: total }, 0]);
    await act(async () => { first.release(); });
    await settled();
    expect([now().loading, now().reading, now().events.length]).toEqual([true, { read: HISTORY_PAGE, of: total }, HISTORY_PAGE]);
    await act(async () => { second.release(); });
    await settled();
    expect([now().loading, now().reading, now().events.length, now().openedAt]).toEqual([false, null, total, total]);
    expect(service.asked.slice(pages).filter(one => one.method === 'session/events').map(one => one.params)).toEqual([
      { from: 0, limit: HISTORY_PAGE }, { from: HISTORY_PAGE, limit: HISTORY_PAGE }, { from: HISTORY_PAGE * 2, limit: 10 },
    ]);
  });

  it('goes on waiting and says nothing of the task whose log it was reading when the service process is started again', async () => {
    const service = new Service();
    const total = HISTORY_PAGE + 10;
    service.logs.set('s-long', long(total));
    const second = held<void>();
    const { now } = await started(service);
    service.on('session/events', async (params) => {
      if (params.from === HISTORY_PAGE) await second.promise;
      return service.page('s-long', params);
    });
    act(() => { void now().openSession('s-long'); });
    await settled();
    expect(now().reading).toEqual({ read: HISTORY_PAGE, of: total });
    act(() => { service.tell('session.stream', { session_id: 's-long', turn: 9, step: 1, chunk: { type: 'text-delta', index: 0, text: 'It is ' } }); });
    expect(now().preview.text).toBe('It is ');
    const starting = held<{ mode: 'service'; cwd: string }>();
    service.bridge.start.mockReturnValueOnce(starting.promise);
    act(() => { void now().reconnect(); });
    await settled();
    // The request for the next page went with the connection. That is nothing to tell the user.
    expect([now().loading, now().reading, now().error, now().connected, now().preview.text]).toEqual([true, null, null, false, '']);
    await act(async () => { starting.release({ mode: 'service', cwd: 'F:\\work' }); });
    await settled();
    await act(async () => { second.release(); });
    await settled();
    // The task that was being read is the one that is opened again.
    expect([now().sessionId, now().loading, now().reading, now().events.length]).toEqual(['s-long', false, null, total]);
  });

  it('says nothing of how far a log of one page is read', async () => {
    const service = new Service();
    service.logs.set('s-page', long(HISTORY_PAGE));
    const page = held<void>();
    const { now } = await started(service);
    service.on('session/events', async (params) => {
      await page.promise;
      return service.page('s-page', params);
    });
    act(() => { void now().openSession('s-page'); });
    await settled();
    expect([now().sessionId, now().loading, now().reading]).toEqual(['s-page', true, null]);
    await act(async () => { page.release(); });
    await settled();
    expect([now().loading, now().reading, now().events.length]).toEqual([false, null, HISTORY_PAGE]);
  });

  describe('while the long log of one that was opened before it is read', () => {
    const total = HISTORY_PAGE + 10;
    /** Two tasks with long logs. The second page of each waits until the test lets it go. */
    const slow = async () => {
      const service = two();
      service.logs.set('s-a', long(total));
      service.logs.set('s-b', long(total));
      const gates = { 's-a': held<void>(), 's-b': held<void>() };
      const page = await started(service);
      let reading = '';
      service.on('session/open', (params) => {
        reading = String(params.session_id);
        return { session_id: reading, events: service.logs.get(reading)!.length };
      });
      service.on('session/events', async (params) => {
        const id = reading;
        if ((id === 's-a' || id === 's-b') && params.from === HISTORY_PAGE) await gates[id].promise;
        return service.page(id, params);
      });
      act(() => { void page.now().openSession('s-a'); });
      await settled();
      expect(page.now().reading).toEqual({ read: HISTORY_PAGE, of: total });
      return { ...page, gates };
    };

    it('takes its place, and the one that was left is not shown when its page comes late', async () => {
      const { now, gates } = await slow();
      await act(async () => { await now().openSession('s-2'); });
      expect([now().sessionId, now().events.length, now().loading, now().reading]).toEqual(['s-2', 2, false, null]);
      await act(async () => { gates['s-a'].release(); });
      await settled();
      expect([now().sessionId, now().events.length, now().loading, now().reading]).toEqual(['s-2', 2, false, null]);
    });

    it('says nothing of what went wrong with the one that was left', async () => {
      const { now, gates } = await slow();
      await act(async () => { await now().openSession('s-2'); });
      await act(async () => { gates['s-a'].refuse(new Error('Too late to matter.')); });
      await settled();
      expect([now().sessionId, now().error]).toEqual(['s-2', null]);
    });

    it('is still being read when the one that was left has come to its end', async () => {
      const { now, gates } = await slow();
      act(() => { void now().openSession('s-b'); });
      await settled();
      expect([now().sessionId, now().loading, now().reading]).toEqual(['s-b', true, { read: HISTORY_PAGE, of: total }]);
      await act(async () => { gates['s-a'].release(); });
      await settled();
      expect([now().sessionId, now().loading, now().reading, now().events.length]).toEqual(['s-b', true, { read: HISTORY_PAGE, of: total }, HISTORY_PAGE]);
      await act(async () => { gates['s-b'].release(); });
      await settled();
      expect([now().loading, now().reading, now().events.length]).toEqual([false, null, total]);
    });
  });

  it('says so where the service process does not go on through the log', async () => {
    const service = two();
    const { now } = await started(service);
    service.on('session/events', () => ({ events: [], next: 0 }));
    await act(async () => { await now().openSession('s-2'); });
    expect([now().error, now().loading]).toEqual(['The service did not advance the history page. Reconnect to try again.', false]);
  });

  it('shows nothing of a log that is longer than the service process said when it opened the task', async () => {
    const service = two();
    const { now } = await started(service);
    // An event was written between the opening and the reading. The page is told of it by itself.
    service.on('session/events', () => ({ events: [...service.logs.get('s-2')!, event(2, reply('Late.'))], next: null }));
    await act(async () => { await now().openSession('s-2'); });
    expect(now().events.map(one => one.seq)).toEqual([0, 1]);
  });

  it('names no skills where the service process knows no such request, and says nothing of it', async () => {
    const service = two();
    service.on('session/skills', () => { throw new Refused(-32601, 'There is no method session/skills.'); });
    const { now } = await started(service);
    expect([now().skills, now().error, now().loading]).toEqual([[], null, false]);
  });

  it('names no skills of the task before while those of the task that is opened are asked for', async () => {
    const service = two();
    service.skills = [skill('review')];
    const { now } = await started(service);
    expect(now().skills).toEqual([skill('review')]);
    const next = held<{ skills: SkillSummary[] }>();
    service.on('session/skills', () => next.promise);
    act(() => { void now().openSession('s-2'); });
    await settled();
    expect([now().sessionId, now().skills]).toEqual(['s-2', []]);
    await act(async () => { next.release({ skills: [skill('notes')] }); });
    await settled();
    expect(now().skills).toEqual([skill('notes')]);
  });

  it('names the skills of the task that is open, and not those that were asked for the task before', async () => {
    const service = two();
    const first = held<{ skills: SkillSummary[] }>();
    let asked = 0;
    service.on('session/skills', () => { asked += 1; return asked === 1 ? first.promise : { skills: [skill('notes')] }; });
    const { now } = mounted(service);
    act(() => { void now().openFolder('F:\\work'); });
    await settled();
    expect([now().sessionId, now().skills]).toEqual(['s-1', []]);
    await act(async () => { await now().openSession('s-2'); });
    expect(now().skills).toEqual([skill('notes')]);
    await act(async () => { first.release({ skills: [skill('review')] }); });
    await settled();
    expect([now().sessionId, now().skills]).toEqual(['s-2', [skill('notes')]]);
  });
});

describe('a service process that ends', () => {
  it('is said to have ended, with why and with what it wrote last', async () => {
    const service = new Service();
    service.logs.set('s-1', log(begin(1), user('Fix the parser.'), call('c-1', 'bash', { command: 'npm test' })));
    service.tail = ['Traceback', 'ValueError: there is no such profile'];
    const { now } = await started(service);
    act(() => {
      service.tell('session.status', statusOf({ status: 'running' }));
      service.ask('q-1', 'approval/ask', asking('s-1', 'c-1'));
      service.tell('session.stream', { session_id: 's-1', turn: 1, step: 1, chunk: { type: 'text-delta', index: 0, text: 'It is ' } });
    });
    await act(async () => { await now().cancel(); });
    expect([now().approval === null, now().preview.text, now().cancelling, now().projection.turn]).toEqual([false, 'It is ', true, 1]);
    act(() => { service.end('It ended with the code 3.'); });
    await settled();
    expect([now().connected, now().loading, now().closed]).toEqual([false, false, 'It ended with the code 3.']);
    // Whatever it was doing went with it.
    expect([now().approval, now().preview.text, now().cancelling, now().status?.status]).toEqual([null, '', false, 'idle']);
    expect(now().lastWords).toEqual(['Traceback', 'ValueError: there is no such profile']);
    // The turn that it left under way is read as one that was cut short.
    expect([now().projection.turn, now().projection.turns.at(-1)?.end?.reason]).toEqual([null, 'interrupted']);
    expect(now().hasProject).toBe(true);
  });

  it('is said to have ended also where no reason is known, and nothing more is sent to it', async () => {
    const service = new Service();
    const { now } = await started(service);
    act(() => { service.tell('session.status', statusOf({ status: 'stopped', detail: 'The log could not be written.' })); });
    act(() => { service.end(''); });
    await settled();
    expect(now().closed).toBe('');
    // What it said last of a task that was not at work is still what is known of the task.
    expect([now().status?.status, now().status?.detail]).toEqual(['stopped', 'The log could not be written.']);
    const before = service.asked.length;
    let taken = true;
    await act(async () => { taken = await now().send('Fix the parser.'); });
    expect(taken).toBe(false);
    expect(service.asked).toHaveLength(before);
    expect(now().error).not.toBeNull();
  });

  it('is told to end when the window is closed, and stopped after it has answered', async () => {
    const service = new Service();
    await started(service);
    const stops = service.bridge.stop.mock.calls.length;
    service.shutDown();
    await settled();
    expect(service.of('shutdown')).toHaveLength(1);
    expect(service.bridge.stop.mock.calls.length).toBe(stops + 1);
  });

  it('is stopped all the same where it does not answer that', async () => {
    const service = new Service();
    service.on('shutdown', () => { throw new Refused(-32000, 'Busy.'); });
    await started(service);
    const stops = service.bridge.stop.mock.calls.length;
    service.shutDown();
    await settled();
    expect(service.bridge.stop.mock.calls.length).toBe(stops + 1);
  });

  it('is told to end once, however often the project was started', async () => {
    const service = new Service();
    const { now } = await started(service);
    await act(async () => { await now().reconnect(); });
    const stops = service.bridge.stop.mock.calls.length;
    service.shutDown();
    await settled();
    expect(service.of('shutdown')).toHaveLength(1);
    expect(service.bridge.stop.mock.calls.length).toBe(stops + 1);
  });

  it('is let go of when the page goes away', async () => {
    const service = new Service();
    const { unmount } = await started(service);
    const stops = service.bridge.stop.mock.calls.length;
    unmount();
    expect(service.bridge.stop.mock.calls.length).toBe(stops + 1);
    // What it says after that is heard by nobody, and nothing is asked of it.
    const asked = service.asked.length;
    service.tell('session.event', telling('s-1', event(0, end(1))));
    service.shutDown();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(service.asked).toHaveLength(asked);
    expect(service.bridge.stop.mock.calls.length).toBe(stops + 1);
  });
});

describe('the services and the folders of the shell', () => {
  it('reads the services as soon as the page is there, before any project is open, and says while it reads them', async () => {
    const service = new Service();
    service.listed = { use: null, services: [listedService('deepseek', 'DeepSeek')] };
    const answer = held<unknown>();
    service.bridge.services.mockReturnValueOnce(answer.promise);
    const { now } = mounted(service);
    expect(now().checkingServices).toBe(true);
    expect(now().services).toBeNull();
    await act(async () => { answer.release(service.listed); });
    expect(now().checkingServices).toBe(false);
    expect(now().services).toEqual(service.listed);
    expect(now().hasProject).toBe(false);
    expect(service.bridge.services).toHaveBeenCalledTimes(1);
  });

  it('asks nothing of a shell that is not there, and is not asking then', () => {
    vi.stubGlobal('columbina', undefined);
    const { result } = renderHook(() => useWorkspace());
    expect(result.current.checkingServices).toBe(false);
  });

  it('holds the answer to the last ask for the services, whichever answer comes first', async () => {
    const service = new Service();
    const first = held<unknown>();
    const second = held<unknown>();
    service.bridge.services.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const { now } = mounted(service);
    let asked: Promise<unknown> = Promise.resolve();
    act(() => { asked = now().refreshServices(); });
    const later = { use: 'openai', services: [listedService('openai', 'OpenAI', { key: 'stored' })] };
    await act(async () => { second.release(later); await asked; });
    expect([now().services, now().checkingServices]).toEqual([later, false]);
    // What the first ask answers, or how it fails, comes too late to count.
    await act(async () => { first.release({ use: null, services: [listedService('deepseek', 'DeepSeek')] }); });
    expect(now().services).toEqual(later);
    const third = held<unknown>();
    service.bridge.services.mockReturnValueOnce(third.promise).mockResolvedValueOnce(later);
    act(() => { void now().refreshServices(); });
    await act(async () => { await now().refreshServices(); });
    await act(async () => { third.refuse(new Error('too late')); });
    expect([now().services, now().checkingServices, now().error]).toEqual([later, false, null]);
  });

  it('reads the services again after one was set up, and says so to whoever asked', async () => {
    const service = new Service();
    service.listed = { use: null, services: [listedService('deepseek', 'DeepSeek')] };
    const { now } = await started(service);
    // Once when the page was there, and once when the service was started.
    expect(service.bridge.services).toHaveBeenCalledTimes(2);
    let stored: unknown = 'not asked';
    await act(async () => { stored = await now().setUpService('deepseek', 'ja'); });
    expect(service.bridge.setUpService.mock.calls).toEqual([[{ service: 'deepseek', language: 'ja' }]]);
    // Nothing was stored in the window of the key.
    expect(stored).toBeNull();
    expect(service.bridge.services).toHaveBeenCalledTimes(2);

    service.bridge.setUpService.mockResolvedValueOnce({ changed: true });
    service.listed = { use: 'deepseek', services: [listedService('deepseek', 'DeepSeek', { key: 'stored' })] };
    await act(async () => { stored = await now().setUpService(null, 'en'); });
    expect(stored).toEqual({ listed: service.listed });
    expect(now().services).toEqual(service.listed);
    expect(service.bridge.services).toHaveBeenCalledTimes(3);
  });

  it('says what went wrong where the window of the key could not be opened', async () => {
    const service = new Service();
    service.bridge.setUpService.mockRejectedValueOnce(new Error('The window could not be opened.'));
    const { now } = await started(service);
    let stored: unknown = 'not asked';
    await act(async () => { stored = await now().setUpService(null, 'en'); });
    expect([stored, now().error]).toEqual([null, 'The window could not be opened.']);
  });

  it('knows of no services where the shell cannot read them, and says why', async () => {
    const service = new Service();
    service.listed = { use: null, services: [listedService('deepseek', 'DeepSeek')] };
    const { now } = await started(service);
    expect(now().services).toEqual(service.listed);
    // A service was stored, and the list cannot be read after it.
    service.bridge.setUpService.mockResolvedValueOnce({ changed: true });
    service.bridge.services.mockRejectedValueOnce(new Error('The program of the service did not answer.'));
    let stored: unknown = 'not asked';
    await act(async () => { stored = await now().setUpService(null, 'en'); });
    expect(stored).toEqual({ listed: null });
    expect([now().services, now().error]).toEqual([null, 'The program of the service did not answer.']);
  });

  it('lets a folder be chosen in the dialog of the system, one dialog at a time', async () => {
    const service = new Service();
    const choosing = held<string | null>();
    service.bridge.selectFolder.mockReturnValueOnce(choosing.promise);
    const { now } = mounted(service);
    let first: Promise<string | null> = Promise.resolve(null);
    let second: string | null = 'not asked';
    act(() => { first = now().pickFolder('ja'); });
    await act(async () => { second = await now().pickFolder('ja'); });
    expect(second).toBeNull();
    expect(service.bridge.selectFolder.mock.calls).toEqual([[{ language: 'ja' }]]);
    await act(async () => { choosing.release('D:\\notes'); });
    expect(await first).toBe('D:\\notes');
    // The dialog is closed: another may be opened.
    await act(async () => { second = await now().pickFolder('en'); });
    expect(service.bridge.selectFolder).toHaveBeenCalledTimes(2);
  });

  it('says what went wrong where the dialog could not be opened', async () => {
    const service = new Service();
    service.bridge.selectFolder.mockRejectedValueOnce(new Error('No dialog.'));
    const { now } = mounted(service);
    let chosen: string | null = 'not asked';
    await act(async () => { chosen = await now().pickFolder('en'); });
    expect([chosen, now().error]).toEqual([null, 'No dialog.']);
    service.bridge.selectFolder.mockResolvedValueOnce('D:\\notes');
    await act(async () => { chosen = await now().pickFolder('en'); });
    expect(chosen).toBe('D:\\notes');
  });

  it('asks the shell for the folder of an earlier project, and reads the list again where that folder is gone', async () => {
    const service = new Service();
    service.bridge.openRecent.mockResolvedValueOnce('F:\\work\\');
    const { now } = mounted(service);
    await settled();
    let folder: string | null = null;
    await act(async () => { folder = await now().pickRecent('F:\\work'); });
    expect(folder).toBe('F:\\work\\');
    expect(service.bridge.openRecent.mock.calls).toEqual([['F:\\work']]);
    const listings = service.bridge.recentProjects.mock.calls.length;
    service.bridge.openRecent.mockRejectedValueOnce(new Error('The folder is gone.'));
    service.recent = [];
    await act(async () => { folder = await now().pickRecent('D:\\notes'); });
    await settled();
    expect([folder, now().error, now().recent]).toEqual([null, 'The folder is gone.', []]);
    expect(service.bridge.recentProjects.mock.calls.length).toBe(listings + 1);
  });

  it('goes on without the folders of earlier days where the shell cannot read them', async () => {
    const service = new Service();
    service.bridge.recentProjects.mockRejectedValue(new Error('The list could not be read.'));
    const { now } = await started(service);
    expect([now().recent, now().error, now().connected]).toEqual([[], null, true]);
  });

  it('shows the log of the service process through the shell', async () => {
    const service = new Service();
    const { now } = mounted(service);
    act(() => { now().showLog(); });
    expect(service.bridge.showLog).toHaveBeenCalledTimes(1);
    service.bridge.showLog.mockRejectedValueOnce(new Error('There is no log yet.'));
    act(() => { now().showLog(); });
    await settled();
    expect(now().error).toBe('There is no log yet.');
  });
});
