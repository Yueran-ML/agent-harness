import { EventEmitter } from 'node:events';
import path from 'node:path';
import { StringDecoder } from 'node:string_decoder';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { EngineWorld, loadEngine, PIPE_A, PIPE_B, settle, type EngineStart } from './engineFakes';

const PROGRAM = path.resolve('/engine/columbina-engine.exe');
const PROJECT = path.resolve('/film/.columbina/studio');
const LOG = path.resolve('/logs/columbina-engine.log');

/**
 * The module of the engine of the shell, with everything around it made up: the programs that it
 * starts, the network, the files, and the system.
 */
function engineHarness(system: {
  platform?: string; env?: Record<string, string>; uid?: number; starts?: EngineStart[]; decoder?: typeof StringDecoder;
} = {}) {
  const world = new EngineWorld();
  world.starts.push(...(system.starts ?? []));
  const files = new Map<string, string>();
  const logs: { file: string; options: unknown; stream: EventEmitter & { end: ReturnType<typeof vi.fn>; write: ReturnType<typeof vi.fn> } }[] = [];
  const fs = {
    createWriteStream: vi.fn((file: string, options: unknown) => {
      const stream = Object.assign(new EventEmitter(), { end: vi.fn(), write: vi.fn() });
      logs.push({ file, options, stream });
      return stream;
    }),
    mkdirSync: vi.fn(),
    readFileSync: (file: string) => {
      const text = files.get(path.resolve(file));
      if (text === undefined) throw Object.assign(new Error(`ENOENT: ${file}`), { code: 'ENOENT' });
      return text;
    },
    realpathSync: (folder: string) => path.resolve(folder),
  };
  const spawn = vi.fn((program: string, args: string[], options: unknown) => world.spawn(program, args, options));
  const kill = vi.fn((child: { pid: number }) => world.kill(child.pid));
  // The processes that are there, by their ids.
  const running = new Set<number>();
  const processKill = (pid: number, _signal: number) => {
    if (!running.has(pid)) throw Object.assign(new Error(`ESRCH: ${pid}`), { code: 'ESRCH' });
  };
  const module = loadEngine(
    {
      'node:child_process': { spawn }, 'node:net': world.net, 'node:fs': fs, 'node:path': path,
      'node:string_decoder': { StringDecoder: system.decoder ?? StringDecoder },
    },
    { process: { platform: system.platform ?? 'win32', getuid: () => system.uid ?? 1000, kill: processKill }, setTimeout, clearTimeout, Date },
  );
  const events: any[] = [];
  const env = system.env ?? { PATH: 'C:\\Windows' };
  const open = () => module.openEngine({ program: PROGRAM, project: PROJECT, env, logFile: LOG, kill, onEvent: (event: unknown) => events.push(event) });
  const lock = (address: unknown) => files.set(path.join(PROJECT, 'engine.lock'), JSON.stringify({ pid: 9, address, started: '2026-10-27T00:00:00Z' }));
  /** The connections that reached an engine, the main one first. */
  const reached = () => world.sockets.filter(socket => socket.server !== null);
  return { world, files, logs, fs, spawn, kill, module, events, env, open, lock, reached, running };
}

afterEach(() => { vi.useRealTimers(); });

describe('starting the engine of a project', () => {
  it('starts it for the project, with a window, and reaches it where its first line says it listens', async () => {
    const engine = engineHarness();
    const session = await engine.open();
    const [child] = engine.world.children;
    expect(engine.spawn).toHaveBeenCalledOnce();
    expect(child.program).toBe(PROGRAM);
    // Without --headless: the engine is the application's, and ends when its stdin is closed.
    expect(child.args).toEqual(['--project', PROJECT]);
    expect(child.options).toEqual({
      cwd: path.dirname(PROGRAM), env: engine.env, windowsHide: true, detached: false, stdio: ['pipe', 'pipe', 'pipe'],
    });
    expect(child.stdin.end).not.toHaveBeenCalled();
    expect(session.owned).toBe(true);
    expect(session.pid).toBe(child.pid);
    expect(session.address).toBe(PIPE_A);
    expect(session.hello).toMatchObject({ protocol: 1, revision: 0, project: PROJECT });
    expect(engine.world.net.connect.mock.calls).toEqual([[PIPE_A], [PIPE_A]]);
    for (const socket of engine.reached()) expect(socket.written).toEqual([{ jsonrpc: '2.0', id: 1, method: 'hello', params: { client: 'desktop', protocol: 1 } }]);
  });

  it('appends what the engine logs to its log as it comes, and never leaves it unread', async () => {
    const engine = engineHarness();
    await engine.open();
    expect(engine.fs.mkdirSync).toHaveBeenCalledWith(path.dirname(LOG), { recursive: true });
    expect(engine.logs).toHaveLength(1);
    expect(engine.logs[0]).toMatchObject({ file: LOG, options: { flags: 'a' } });
    const [child] = engine.world.children;
    const [{ stream }] = engine.logs;
    expect(stream.write).toHaveBeenCalledWith(Buffer.from('{"event": "proxy_ffmpeg", "path": null}\n', 'utf8'));
    // A log that fails is let go of, and what the engine writes is still read.
    stream.emit('error', new Error('ENOSPC'));
    child.stderr.emit('data', Buffer.from('{"event": "view_ready"}\n', 'utf8'));
    expect(stream.write).toHaveBeenCalledOnce();
    expect(child.stderr.listenerCount('data')).toBe(1);
    // What it writes on stdout after its first line, which is nothing, would not hold it up either.
    expect(child.stdout.listenerCount('data')).toBe(1);
  });

  it('refuses a first line that is not of this protocol, or names no address of an engine, and ends that engine', async () => {
    for (const first of [
      'not json', '{"listening": "\\\\\\\\.\\\\pipe\\\\columbina-engine-0123456789abcdef", "pid": 5, "protocol": 2}',
      '{"listening": "\\\\\\\\.\\\\pipe\\\\somebody-else", "pid": 5, "protocol": 1}',
      '{"listening": "\\\\\\\\server\\\\pipe\\\\columbina-engine-0123456789abcdef", "pid": 5, "protocol": 1}',
    ]) {
      const engine = engineHarness({ starts: [{ first }] });
      await expect(engine.open()).rejects.toThrow('did not say where it listens in protocol 1');
      expect(engine.kill).toHaveBeenCalledWith(engine.world.children[0]);
      expect(engine.world.net.connect).not.toHaveBeenCalled();
    }
  });

  it('gives an engine 20 seconds to say where it listens', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const engine = engineHarness({ starts: ['silent'] });
    const opening = engine.open();
    const failed = expect(opening).rejects.toThrow('did not say where it listens within 20 seconds');
    await vi.advanceTimersByTimeAsync(19_999);
    expect(engine.kill).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    await failed;
    expect(engine.kill).toHaveBeenCalledWith(engine.world.children[0]);
  });

  it('says what the engine said where it could not start, and does not try again', async () => {
    const engine = engineHarness({ starts: [{ error: 'the address is held by another program' }] });
    await expect(engine.open()).rejects.toThrow('the address is held by another program');
    expect(engine.spawn).toHaveBeenCalledOnce();
  });

  it('says how an engine ended that ended before it listened, and names the libraries that it misses', async () => {
    const ended = engineHarness({ starts: [{ exit: 1, stderr: '{"event": "kicked"}' }] });
    await expect(ended.open()).rejects.toThrow(`The engine program ${PROGRAM} ended with exit code 1 before it listened.`);
    const missing = engineHarness({ starts: [{ exit: 0xC0000135 }] });
    await expect(missing.open()).rejects.toThrow(/exit code 0xC0000135 .*avcodec, avformat, avutil, swscale/);
  });

  it('reaches the engine that has the project open where it said it listens', async () => {
    const engine = engineHarness({ starts: [{ held: PIPE_B }] });
    const agents = engine.world.serve(PIPE_B, PROJECT, 31);
    const session = await engine.open();
    expect(session.owned).toBe(false);
    expect(session.pid).toBe(31);
    expect(engine.world.net.connect.mock.calls).toEqual([[PIPE_B], [PIPE_B]]);
    expect(agents.clients.size).toBe(2);
    // Closing it lets go of the connections, and of nothing else: that engine is its agent's.
    await session.close();
    await settle();
    expect(agents.sockets.every(socket => socket.destroyed)).toBe(true);
    expect(engine.events).toEqual([{ kind: 'exited', expected: true, code: null, signal: null }]);
  });

  it('never opens an address that is not an engine\'s, and looks in engine.lock instead', async () => {
    for (const address of ['\\\\server\\pipe\\columbina-engine-0123456789abcdef', '\\\\.\\pipe\\columbina-engine-0123456789ABCDEF', 'C:\\x', 7, null]) {
      const engine = engineHarness({ starts: [{ held: address }] });
      engine.world.serve(PIPE_B, PROJECT, 31);
      engine.lock(PIPE_B);
      await expect(engine.open()).resolves.toMatchObject({ owned: false, address: PIPE_B });
      expect(engine.world.net.connect.mock.calls).toEqual([[PIPE_B], [PIPE_B]]);
    }
  });

  it('reads engine.lock again a moment later where the engine that held the project does not answer', async () => {
    const engine = engineHarness({ starts: [{ held: PIPE_A }] });
    // The engine at PIPE_A has just ended; the one that took the project after it listens at PIPE_B.
    engine.world.serve(PIPE_B, PROJECT, 32);
    engine.lock(PIPE_B);
    const began = Date.now();
    await expect(engine.open()).resolves.toMatchObject({ owned: false, pid: 32 });
    expect(Date.now() - began).toBeGreaterThanOrEqual(45);
    expect(engine.world.net.connect.mock.calls.map(([address]) => address)).toEqual([PIPE_A, PIPE_B, PIPE_B]);
  });

  it('starts it again where neither answers, for 10 seconds', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const engine = engineHarness({ starts: [{ contended: true }, ...Array.from({ length: 400 }, () => ({ held: PIPE_B }))] });
    engine.lock('\\\\other\\pipe\\columbina-engine-0123456789abcdef');
    const opening = engine.open();
    const failed = expect(opening).rejects.toThrow(`No engine for the project ${PROJECT} could be reached: the engine that has it open did not answer.`);
    await vi.advanceTimersByTimeAsync(9_000);
    const started = engine.spawn.mock.calls.length;
    expect(started).toBeGreaterThan(40);
    await vi.advanceTimersByTimeAsync(1_300);
    await failed;
    expect(engine.spawn.mock.calls.length).toBeLessThan(started + 15);
    vi.useRealTimers();
    // An engine that was still starting is found once it has.
    const later = engineHarness({ starts: [{ contended: true }] });
    later.world.serve(PIPE_A, PROJECT);
    later.lock(PIPE_A);
    await expect(later.open()).resolves.toMatchObject({ owned: false });
  });

  it('refuses an engine that has another project open', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const engine = engineHarness({ starts: Array.from({ length: 400 }, () => ({ held: PIPE_B })) });
    engine.world.serve(PIPE_B, path.resolve('/another/.columbina/studio'));
    const failed = expect(engine.open()).rejects.toThrow('has another project open');
    await vi.advanceTimersByTimeAsync(10_500);
    await failed;
  });

  it('has a headless engine that holds the project yield it, waits until it has ended, and starts its own', async () => {
    const engine = engineHarness({ starts: [{ held: PIPE_B }] });
    const agents = engine.world.headless(PIPE_B, PROJECT, 31, engine.running);
    const session = await engine.open();
    expect(agents.sockets[0].methods()).toEqual(['hello', 'engine.yield']);
    expect(engine.events[0]).toEqual({ kind: 'taking-over', pid: 31 });
    // Its own, with a window, started once that one was gone.
    expect(session.owned).toBe(true);
    expect(session.address).toBe(PIPE_A);
    expect(engine.spawn).toHaveBeenCalledTimes(2);
    expect(engine.running.has(31)).toBe(false);
  });

  it('waits ten seconds at most for a headless engine to end, and asks once', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const engine = engineHarness({ starts: [{ held: PIPE_B }, { held: PIPE_B }] });
    const agents = engine.world.headless(PIPE_B, PROJECT, 31, engine.running);
    // It answers, and does not end.
    agents.answers.set('engine.yield', () => ({ result: {} }));
    const opening = engine.open();
    await vi.advanceTimersByTimeAsync(9_900);
    expect(engine.spawn).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(300);
    const session = await opening;
    // Then it is reached as it is, without a view.
    expect(session.owned).toBe(false);
    expect(engine.spawn).toHaveBeenCalledTimes(2);
    expect(agents.sockets.flatMap(socket => socket.methods()).filter(method => method === 'engine.yield')).toHaveLength(1);
  });

  it('reaches an engine that does not yield as it is', async () => {
    const engine = engineHarness({ starts: [{ held: PIPE_B }] });
    const agents = engine.world.headless(PIPE_B, PROJECT, 31, engine.running);
    agents.answers.set('engine.yield', () => ({ error: { code: -32601, message: 'no such method' } }));
    const session = await engine.open();
    expect(session.owned).toBe(false);
    expect(session.hello.headless).toBe(true);
    expect(engine.spawn).toHaveBeenCalledOnce();
  });

  it('takes on Unix only a socket of the engine\'s name in the directory where this user\'s engines make them', () => {
    const rules = (env: Record<string, string>) => ({ platform: 'linux', env, uid: 501 });
    const { module } = engineHarness();
    const at = (address: unknown, env: Record<string, string> = {}) => module.engineAt(address, rules(env));
    expect(at('/tmp/columbina-501/0123456789abcdef.sock')).toBe(true);
    expect(at('/var/folders/x/columbina-501/0123456789abcdef.sock', { TMPDIR: '/var/folders/x' })).toBe(true);
    expect(at('/run/user/501/columbina/0123456789abcdef.sock', { XDG_RUNTIME_DIR: '/run/user/501', TMPDIR: '/var/x' })).toBe(true);
    expect(at('/tmp/columbina-501/0123456789abcdef.sock', { XDG_RUNTIME_DIR: '/run/user/501' })).toBe(false);
    expect(at('/tmp/columbina-502/0123456789abcdef.sock')).toBe(false);
    expect(at('/tmp/columbina-501/0123456789ABCDEF.sock')).toBe(false);
    expect(at('/tmp/columbina-501/sub/0123456789abcdef.sock')).toBe(false);
    expect(at('/tmp/columbina-501/0123456789abcdef.sock', { XDG_RUNTIME_DIR: 'relative' })).toBe(true);
    expect(at(PIPE_A)).toBe(false);
    // And on Windows a pipe of this machine of the engine's name, and nothing else.
    expect(module.engineAt(PIPE_A, { platform: 'win32', env: {}, uid: 0 })).toBe(true);
    expect(module.engineAt('/tmp/columbina-501/0123456789abcdef.sock', { platform: 'win32', env: {}, uid: 0 })).toBe(false);
  });
});

describe('a session with the engine', () => {
  it('asks on the main connection, and on one of its own for the view', async () => {
    const engine = engineHarness();
    const session = await engine.open();
    const [main, view] = engine.reached();
    const server = engine.world.servers.get(PIPE_A)!;
    server.answers.set('project.get', () => ({ result: { revision: 0, next_id: 1, project: { schema: 1, next_id: 1, media: [], timelines: [] } } }));
    server.answers.set('view.attach', () => 'hold');
    const attaching = session.view('view.attach', { parent: '1', x: 0, y: 0, width: 1, height: 1 });
    // The main connection answers while the view is being made.
    await expect(session.call('project.get')).resolves.toMatchObject({ revision: 0 });
    expect(main.methods()).toEqual(['hello', 'project.get']);
    expect(view.methods()).toEqual(['hello', 'view.attach']);
    // A method that takes nothing is sent without params.
    expect(main.written.at(-1)).toEqual({ jsonrpc: '2.0', id: 2, method: 'project.get' });
    server.answerHeld('view.attach', { result: { adapter: 'A', backend: 'Dx12' } });
    await expect(attaching).resolves.toEqual({ adapter: 'A', backend: 'Dx12' });
  });

  it('gives an error of the engine with its code and data, and the connection goes on', async () => {
    const engine = engineHarness();
    const session = await engine.open();
    engine.world.servers.get(PIPE_A)!.answers.set('transport.play', () => ({ error: { code: -32004, message: 'no media is loaded', data: { reason: 'not_loaded' } } }));
    await expect(session.call('transport.play', { speed: 1 })).rejects.toMatchObject({ message: 'no media is loaded', code: -32004, data: { reason: 'not_loaded' } });
    await expect(session.call('transport.state')).resolves.toEqual({});
  });

  it('matches each answer to its request, an answer with no id to the first that waits, and lines that come in pieces', async () => {
    const engine = engineHarness();
    const session = await engine.open();
    const [main] = engine.reached();
    const server = engine.world.servers.get(PIPE_A)!;
    server.answers.set('transport.seek', () => 'hold');
    server.answers.set('transport.step', () => 'hold');
    const seeking = session.call('transport.seek', { frame: 1 });
    const stepping = session.call('transport.step', { frames: 1 });
    main.emit('data', Buffer.from('{"jsonrpc":"2.0","id":3,"result":{"st', 'utf8'));
    main.emit('data', Buffer.from('epped":true}}\r\n\n{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"request line exceeds 16 MiB"}}\n', 'utf8'));
    await expect(stepping).resolves.toEqual({ stepped: true });
    await expect(seeking).rejects.toMatchObject({ code: -32600 });
  });

  it('refuses a request longer than the engine takes before it is sent', async () => {
    const engine = engineHarness();
    const session = await engine.open();
    const [main] = engine.reached();
    await expect(session.call('transaction.apply', { ops: [{ op: 'add_media', name: 'x'.repeat(16 * 1024 * 1024) }] }))
      .rejects.toMatchObject({ code: -32600 });
    expect(main.methods()).toEqual(['hello']);
  });

  it('hands on what the engine tells, in the order it came, once though both connections hear it', async () => {
    const engine = engineHarness();
    const session = await engine.open();
    const server = engine.world.servers.get(PIPE_A)!;
    server.answers.set('transaction.apply', () => {
      // A client is told of its own change before it is answered.
      server.notify('project.changed', { revision: 1, by: 'desktop', made_by: 'desktop', done: [], next_id: 2 });
      return { result: { revision: 1, done: [], next_id: 2, names: {} } };
    });
    server.notify('transport.state', { media: null, frame: null });
    const applying = session.call('transaction.apply', { ops: [] });
    server.notify('media.proxy', { media: 'm1', state: 'queued' });
    await applying;
    expect(engine.events).toEqual([
      { kind: 'notification', method: 'transport.state', params: { media: null, frame: null } },
      { kind: 'notification', method: 'project.changed', params: { revision: 1, by: 'desktop', made_by: 'desktop', done: [], next_id: 2 } },
      { kind: 'notification', method: 'media.proxy', params: { media: 'm1', state: 'queued' } },
    ]);
  });

  it('is gone when its engine ends, and says how, once; whatever waits is refused', async () => {
    const engine = engineHarness();
    const session = await engine.open();
    engine.world.servers.get(PIPE_A)!.answers.set('project.get', () => 'hold');
    const asking = session.call('project.get');
    engine.world.children[0].end(1);
    await expect(asking).rejects.toThrow('The connection to the engine was closed.');
    await settle();
    expect(engine.events).toEqual([{ kind: 'exited', expected: false, code: 1, signal: null }]);
    expect(session.alive).toBe(false);
    await expect(session.call('project.get')).rejects.toThrow('closed');
  });

  it('ends the engine that it started when a connection to it is lost', async () => {
    const engine = engineHarness();
    await engine.open();
    // The engine closes a connection that does not read; the session ends rather than be left half.
    engine.reached()[1].destroy();
    await settle();
    expect(engine.world.children[0].stdin.end).toHaveBeenCalledOnce();
    expect(engine.events).toEqual([{ kind: 'exited', expected: false, code: 0, signal: null }]);
  });

  it('closes the connections and the stdin of its engine, and kills it where it has not ended 5 seconds later', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const engine = engineHarness();
    const opening = engine.open();
    await vi.advanceTimersByTimeAsync(1);
    const session = await opening;
    const [child] = engine.world.children;
    child.stubborn = true;
    const closing = session.close();
    await vi.advanceTimersByTimeAsync(1);
    expect(engine.reached().every(socket => socket.destroyed)).toBe(true);
    expect(child.stdin.end).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(4_998);
    expect(engine.kill).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2);
    expect(engine.kill).toHaveBeenCalledWith(child);
    await closing;
    expect(engine.events).toEqual([{ kind: 'exited', expected: true, code: 1, signal: null }]);
    // Closed twice is closed once.
    await session.close();
    expect(engine.events).toHaveLength(1);
  });

  it('ends the session where a line from the engine cannot be held as a string', async () => {
    // A decoder that fails as V8 does with a string longer than it holds.
    class Overlong extends StringDecoder {
      write(chunk: Buffer) {
        if (chunk.toString('utf8') === 'TOO LONG') throw new RangeError('Invalid string length');
        return super.write(chunk);
      }
    }
    const engine = engineHarness({ decoder: Overlong as unknown as typeof StringDecoder });
    const session = await engine.open();
    engine.world.servers.get(PIPE_A)!.answers.set('project.get', () => 'hold');
    const asking = session.call('project.get');
    engine.reached()[0].emit('data', Buffer.from('TOO LONG', 'utf8'));
    await expect(asking).rejects.toThrow('A line from the engine could not be read: Invalid string length');
    await settle();
    expect(engine.events.at(-1)).toMatchObject({ kind: 'exited', expected: false });
    expect(engine.world.children[0].stdin.end).toHaveBeenCalled();
  });

  it('lets go of everything at once where the application ends', async () => {
    const engine = engineHarness();
    const session = await engine.open();
    session.abandon();
    expect(engine.world.children[0].stdin.end).toHaveBeenCalled();
    expect(engine.reached().every(socket => socket.destroyed)).toBe(true);
  });
});
