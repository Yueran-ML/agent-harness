import { EventEmitter } from 'node:events';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { runInNewContext } from 'node:vm';
import { vi } from 'vitest';

/** Where engines listen in the tests: pipes of Windows of the engine's name. */
export const PIPE_A = '\\\\.\\pipe\\columbina-engine-0123456789abcdef';
export const PIPE_B = '\\\\.\\pipe\\columbina-engine-fedcba9876543210';

const enginePath = fileURLToPath(new URL('../electron/engine.cjs', import.meta.url));
const engineSource = readFileSync(enginePath, 'utf8');

/** The module of the engine of the shell, run with the modules and the globals that are given. */
export function loadEngine(modules: Record<string, unknown>, globals: Record<string, unknown>) {
  const module = { exports: {} as Record<string, any> };
  runInNewContext(engineSource, {
    require: (name: string) => { if (!(name in modules)) throw new Error(`Unexpected module: ${name}`); return modules[name]; },
    module, exports: module.exports, Buffer, ...globals,
  }, { filename: enginePath });
  return module.exports;
}

/** A message of JSON-RPC, as an engine writes it. */
export interface Said {
  jsonrpc?: string;
  id?: number | string | null;
  method?: string;
  params?: any;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

/** A connection of a client to an engine, as the client holds it. */
export class FakeSocket extends EventEmitter {
  destroyed = false;
  /** The lines that the client wrote, as messages. */
  written: Said[] = [];
  server: FakeServer | null = null;
  constructor(public address: string) { super(); }
  write = vi.fn((text: string) => {
    for (const line of text.split('\n').filter(Boolean)) {
      const message = JSON.parse(line) as Said;
      this.written.push(message);
      this.server?.heard(this, message);
    }
    return true;
  });
  destroy = vi.fn(() => {
    if (this.destroyed) return;
    this.destroyed = true;
    this.server?.clients.delete(this);
    queueMicrotask(() => this.emit('close'));
  });
  /** Hand the client a line as the engine writes it, maybe in pieces. */
  tell(message: Said | string) {
    const line = typeof message === 'string' ? message : JSON.stringify({ jsonrpc: '2.0', ...message });
    this.emit('data', Buffer.from(`${line}\n`, 'utf8'));
  }
  /** The requests that the client sent here, by their methods. */
  methods() { return this.written.map(message => message.method); }
}

/** An answer that a test gives a method by hand: a result, or an error of the engine. */
export type Answer = { result: unknown } | { error: { code: number; message: string; data?: unknown } } | 'hold';

/** An engine that listens at an address and answers what it is asked. */
export class FakeServer {
  revision = 0;
  /** Every connection that said hello. */
  clients = new Set<FakeSocket>();
  /** Every connection that was made to it. */
  sockets: FakeSocket[] = [];
  /** What each method is answered, by its name; `{}` where it is not named. */
  answers = new Map<string, (params: any, socket: FakeSocket) => Answer>();
  /** Requests that were held, to be answered by the test. */
  held: { socket: FakeSocket; message: Said }[] = [];
  /** Whether it was started without a window, as an agent's is. */
  headless = false;
  constructor(public address: string, public project: string, public pid = 4321) {}
  accept(socket: FakeSocket) {
    socket.server = this;
    this.sockets.push(socket);
  }
  heard(socket: FakeSocket, message: Said) {
    let answer: Answer;
    if (message.method === 'hello') {
      this.clients.add(socket);
      answer = { result: { engine: '0.1.0', protocol: 1, revision: this.revision, project: this.project, pid: this.pid, headless: this.headless } };
    } else {
      answer = this.answers.get(message.method!)?.(message.params, socket) ?? { result: {} };
    }
    if (answer === 'hold') {
      this.held.push({ socket, message });
      return;
    }
    queueMicrotask(() => { if (!socket.destroyed) socket.tell({ id: message.id, ...answer as object }); });
  }
  /** Answer a request that was held. */
  answerHeld(method: string, answer: Exclude<Answer, 'hold'>) {
    const at = this.held.findIndex(each => each.message.method === method);
    const [{ socket, message }] = this.held.splice(at, 1);
    socket.tell({ id: message.id, ...answer as object });
  }
  /** Tell every connection that said hello. */
  notify(method: string, params: unknown) {
    for (const socket of this.clients) socket.tell({ method, params });
  }
  /** Close every connection, as an engine that ends does. */
  close() {
    for (const socket of [...this.sockets]) socket.destroy();
  }
}

/** How a program of the engine that is started behaves. */
export type EngineStart =
  | { listen: string }
  | { held: unknown }
  | { contended: true }
  | { error: string }
  | { exit: number; stderr?: string }
  | { first: string }
  | 'silent';

/** A program of the engine, as the shell starts it. */
export interface FakeChild extends EventEmitter {
  pid: number;
  program: string;
  args: string[];
  options: any;
  exitCode: number | null;
  signalCode: string | null;
  stdout: EventEmitter;
  stderr: EventEmitter & { pipe: ReturnType<typeof vi.fn> };
  stdin: EventEmitter & { end: ReturnType<typeof vi.fn> };
  /** Whether it ends when its stdin is closed. */
  stubborn: boolean;
  /** End it, as the system ends it. */
  end(code: number | null, signal?: string | null): void;
}

/**
 * The engines of the tests: the network that reaches them, and the programs that are started.
 * Each start takes the next of `starts`; without one, it listens at PIPE_A.
 */
export class EngineWorld {
  servers = new Map<string, FakeServer>();
  sockets: FakeSocket[] = [];
  children: FakeChild[] = [];
  starts: EngineStart[] = [];
  /** Addresses at which a connection is refused once more, each time it is tried. */
  refusing = new Map<string, number>();
  net = {
    connect: vi.fn((address: string) => {
      const socket = new FakeSocket(address);
      this.sockets.push(socket);
      queueMicrotask(() => {
        const server = this.servers.get(address);
        const refused = this.refusing.get(address) ?? 0;
        if (!server || refused > 0) {
          if (refused > 0) this.refusing.set(address, refused - 1);
          socket.emit('error', Object.assign(new Error(`ENOENT: ${address}`), { code: 'ENOENT' }));
          socket.destroy();
          return;
        }
        server.accept(socket);
        socket.emit('connect');
      });
      return socket;
    }),
  };

  /** An engine that listens at an address with a project open, as another program started it. */
  serve(address: string, project: string, pid = 4321) {
    const server = new FakeServer(address, project, pid);
    this.servers.set(address, server);
    return server;
  }

  /** Start a program of the engine, as `spawn` does. */
  spawn(program: string, args: string[], options: any): FakeChild {
    const start = this.starts.shift() ?? { listen: PIPE_A };
    const child = Object.assign(new EventEmitter(), {
      pid: 600 + this.children.length, program, args, options, exitCode: null as number | null, signalCode: null as string | null,
      stdout: new EventEmitter(), stderr: Object.assign(new EventEmitter(), { pipe: vi.fn() }),
      stdin: Object.assign(new EventEmitter(), { end: vi.fn() }), stubborn: false,
    }) as unknown as FakeChild;
    let server: FakeServer | null = null;
    child.end = (code, signal = null) => {
      if (child.exitCode !== null || child.signalCode !== null) return;
      if (server) {
        this.servers.delete(server.address);
        server.close();
      }
      child.exitCode = code;
      child.signalCode = signal;
      queueMicrotask(() => child.emit('close', code, signal));
    };
    child.stdin.end.mockImplementation(() => { if (!child.stubborn) child.end(0); });
    this.children.push(child);
    const project = args[args.indexOf('--project') + 1];
    const errors = (line: string) => child.stderr.emit('data', Buffer.from(`${line}\n`, 'utf8'));
    queueMicrotask(() => {
      if (start === 'silent') return;
      if ('listen' in start) {
        server = this.serve(start.listen, project, child.pid);
        errors('{"event": "proxy_ffmpeg", "path": null}');
        child.stdout.emit('data', Buffer.from(`{"listening": ${JSON.stringify(start.listen)}, "pid": ${child.pid}, "protocol": 1}\n`, 'utf8'));
      } else if ('first' in start) {
        child.stdout.emit('data', Buffer.from(`${start.first}\n`, 'utf8'));
      } else if ('held' in start) {
        errors(JSON.stringify({ held: { pid: 77, address: start.held } }));
        child.end(3);
      } else if ('contended' in start) {
        errors('{"contended": true}');
        child.end(3);
      } else if ('error' in start) {
        errors(JSON.stringify({ error: start.error }));
        child.end(2);
      } else {
        if (start.stderr) errors(start.stderr);
        child.end(start.exit);
      }
    });
    return child;
  }

  /** The engine that a program of the tests started, by the program's process. */
  serverOf(child: FakeChild) {
    return [...this.servers.values()].find(server => server.pid === child.pid) ?? null;
  }

  /**
   * An engine of an agent, without a window, that has the project open, and yields it when it is
   * asked: it tells its clients, answers, closes every connection and ends. `running` holds the
   * processes that are there, and loses this one as it ends.
   */
  headless(address: string, project: string, pid: number, running: Set<number>) {
    const server = this.serve(address, project, pid);
    server.headless = true;
    running.add(pid);
    server.answers.set('engine.yield', () => {
      server.notify('engine.yielding', { to: 'desktop' });
      queueMicrotask(() => {
        this.servers.delete(address);
        server.close();
        running.delete(pid);
      });
      return { result: {} };
    });
    return server;
  }

  /** End a program as `taskkill` or a signal ends it. */
  kill(pid: number) {
    this.children.find(child => child.pid === pid)?.end(1);
  }
}

/** Let what is under way come about: answers, connections and the ends of programs. */
export async function settle(times = 20) {
  for (let at = 0; at < times; at += 1) await new Promise(resolve => setImmediate(resolve));
}
