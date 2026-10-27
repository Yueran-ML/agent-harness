// The engine of the studio, as the main process starts it and talks to it. See engine/PROTOCOL.md.
//
// The main process starts `columbina-engine --project <dir>` without `--headless`: the engine is
// its own, keeps its stdin open while it lives, and ends when that is closed. Where another engine
// has the project open already (an agent's, without a window), it is reached there instead. Each
// engine is reached on two connections: one for everything, and one of its own for the view, whose
// `view.attach` may take long.
const { spawn } = require('node:child_process');
const net = require('node:net');
const { createWriteStream, mkdirSync, readFileSync, realpathSync } = require('node:fs');
const path = require('node:path');
const { StringDecoder } = require('node:string_decoder');

const PROTOCOL = 1;
/** How long an engine is given to say where it listens, in milliseconds. */
const LISTENING_MS = 20_000;
/** How long a connection is given to be made: a busy pipe of Windows is tried again for ever. */
const CONNECT_MS = 5_000;
/** How long an engine is given to end once its stdin is closed, and once it was killed. */
const CLOSE_MS = 5_000;
const KILL_MS = 2_000;
/** How often an engine is started, or the one that holds the project looked for, before giving up. */
const TRIES = 5;
/** The longest request line that an engine takes, without its newline. */
const LINE_MOST = 16 * 1024 * 1024;
/** How much is kept of what an engine wrote on stderr before it listened: so many lines, of so many characters. */
const TAIL_LINES = 20;
const TAIL_LINE_CHARS = 2000;
/** How long the first line on stdout may be. It is a short object. */
const FIRST_LINE_MOST = 64 * 1024;
/** The exit codes of an engine that did not start: it could not, or another engine has the project open. */
const EXIT_CANNOT_START = 2;
const EXIT_HELD = 3;
/** The exit code of a program of Windows that misses a library it was linked with. */
const LIBRARIES_MISSING = 0xC0000135;

/** The addresses at which an engine listens: a pipe of this machine, or a socket of this user. */
const PIPE = /^\\\\\.\\pipe\\columbina-engine-[0-9a-f]{16}$/;
const SOCKET = /^[0-9a-f]{16}\.sock$/;

/** An error of the engine, with its code and data where the engine gave them. */
function failure(message, code = null, data = undefined) {
  return Object.assign(new Error(message), { code, data });
}

/**
 * The directory in which an engine makes its sockets on Unix, as it chooses it from the
 * environment that it was started with.
 */
function socketsOf(env, uid) {
  const runtime = env.XDG_RUNTIME_DIR;
  if (runtime && path.posix.isAbsolute(runtime)) return path.posix.join(runtime, 'columbina');
  const temporary = env.TMPDIR;
  const base = temporary && path.posix.isAbsolute(temporary) ? temporary : '/tmp';
  return path.posix.join(base, `columbina-${uid}`);
}

/**
 * Whether an address is one at which an engine listens, and none other: a project, and its
 * `engine.lock`, may come from anywhere. On Windows a pipe of this machine of the engine's name,
 * never a pipe of another machine, to which Windows would sign in; elsewhere a socket of that name
 * in the directory of this user where the engine makes its sockets.
 */
function engineAt(address, rules) {
  if (typeof address !== 'string') return false;
  if (rules.platform === 'win32') return PIPE.test(address);
  return SOCKET.test(path.posix.basename(address)) && path.posix.dirname(address) === socketsOf(rules.env, rules.uid);
}

/** The address that `engine.lock` of a project names, if it names one at which an engine listens. */
function locked(project, rules) {
  let said = null;
  try { said = JSON.parse(readFileSync(path.join(project, 'engine.lock'), 'utf8')); } catch { return null; }
  const address = said !== null && typeof said === 'object' ? said.address : null;
  return engineAt(address, rules) ? address : null;
}

/** Wait the moment after which an address that did not answer is looked up again: 50 to 100 ms. */
const moment = () => new Promise((resolve) => { setTimeout(resolve, 50 + Math.floor(Math.random() * 51)); });

/** Whether two paths name one directory, as the system finds them. */
function sameDirectory(one, other, platform) {
  try {
    const key = (folder) => {
      const found = realpathSync(folder);
      return platform === 'win32' ? found.toLowerCase() : found;
    };
    return typeof one === 'string' && key(one) === key(other);
  } catch {
    return false;
  }
}

/** Open a connection to an address. Null where none was made in time. */
function connectTo(address) {
  return new Promise((resolve) => {
    let socket;
    try {
      socket = net.connect(address);
    } catch {
      resolve(null);
      return;
    }
    let settled = false;
    const settle = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.removeListener('error', failed);
      if (value === null) socket.destroy();
      resolve(value);
    };
    const failed = () => settle(null);
    const timer = setTimeout(failed, CONNECT_MS);
    socket.once('error', failed);
    socket.once('connect', () => settle(socket));
  });
}

/**
 * A connection to an engine: one message of JSON-RPC a line each way. Answers are matched to
 * their requests; notifications are handed on in the order they came. Once it is gone, every
 * request that waits is refused, and `gone` is called once.
 */
function wireOf(socket, { notified, gone }) {
  const decoder = new StringDecoder('utf8');
  const pending = new Map();
  let pieces = [];
  let number = 0;
  let over = false;
  const heard = (line) => {
    const text = line.endsWith('\r') ? line.slice(0, -1) : line;
    if (text.trim() === '') return;
    let said = null;
    try { said = JSON.parse(text); } catch { return; }
    if (said === null || typeof said !== 'object' || Array.isArray(said)) return;
    if (Object.hasOwn(said, 'id') && (Object.hasOwn(said, 'result') || Object.hasOwn(said, 'error'))) {
      // Requests are answered in the order they were sent: one that the engine could not read
      // whole is answered with no id, and is the first that waits.
      const id = said.id === null ? pending.keys().next().value : said.id;
      const asked = pending.get(id);
      if (!asked) return;
      pending.delete(id);
      if (said.error) asked.reject(failure(String(said.error.message ?? 'The engine refused.'), said.error.code ?? null, said.error.data));
      else asked.resolve(said.result);
    } else if (typeof said.method === 'string') {
      notified(said.method, said.params ?? null);
    }
  };
  socket.on('data', (chunk) => {
    // Only what came now is looked through for the end of a line: a project may be a long one.
    const text = decoder.write(chunk);
    let from = 0;
    let at = text.indexOf('\n');
    while (at >= 0) {
      pieces.push(text.slice(from, at));
      const line = pieces.join('');
      pieces = [];
      heard(line);
      from = at + 1;
      at = text.indexOf('\n', from);
    }
    if (from < text.length) pieces.push(text.slice(from));
  });
  const lost = () => {
    if (over) return;
    over = true;
    for (const asked of pending.values()) asked.reject(failure('The connection to the engine was closed.'));
    pending.clear();
    gone();
  };
  socket.on('error', lost);
  socket.on('close', lost);
  return {
    call(method, params) {
      if (over) return Promise.reject(failure('The connection to the engine was closed.'));
      number += 1;
      const id = number;
      const message = { jsonrpc: '2.0', id, method };
      if (params !== undefined && params !== null) message.params = params;
      const line = JSON.stringify(message);
      if (Buffer.byteLength(line, 'utf8') > LINE_MOST) {
        return Promise.reject(failure('The request is longer than the engine takes (16 MiB).', -32600));
      }
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        socket.write(`${line}\n`, 'utf8');
      });
    },
    close() {
      lost();
      socket.destroy();
    },
  };
}

/** Say hello as the desktop, and check that the engine there has this project open. */
async function greet(wire, project, platform) {
  const hello = await wire.call('hello', { client: 'desktop', protocol: PROTOCOL });
  if (hello?.protocol !== PROTOCOL) throw failure(`The engine speaks protocol ${hello?.protocol}, and this application ${PROTOCOL}.`);
  if (!sameDirectory(hello.project, project, platform)) {
    throw failure(`The engine at that address has another project open: ${hello.project}`);
  }
  return hello;
}

/**
 * Wait until a program of the engine has ended: its stdin is closed, which it takes for a stop,
 * and it is killed where it has not ended after a few seconds.
 */
function stopChild(started, kill) {
  if (started.ended) return started.closed;
  try { started.child.stdin.end(); } catch { /* It has ended. */ }
  const timers = [];
  // One that does not end even when killed is not waited for longer.
  const given = new Promise((resolve) => {
    timers.push(setTimeout(() => {
      kill(started.child);
      timers.push(setTimeout(() => resolve(null), KILL_MS));
    }, CLOSE_MS));
  });
  return Promise.race([started.closed, given]).finally(() => timers.forEach((timer) => clearTimeout(timer)));
}

/** What an engine that ended before it listened said of why: held by another, or not started. */
function endedWith(program, code, tail) {
  for (const line of [...tail].reverse()) {
    let said = null;
    try { said = JSON.parse(line); } catch { continue; }
    if (said === null || typeof said !== 'object' || Array.isArray(said)) continue;
    if (code === EXIT_HELD && said.held !== null && typeof said.held === 'object') return { kind: 'held', address: said.held.address };
    if (code === EXIT_HELD && said.contended === true) return { kind: 'contended', address: null };
    if (code === EXIT_CANNOT_START && typeof said.error === 'string') throw failure(said.error);
  }
  const shown = code !== null && code > 0xFFFF ? `0x${(code >>> 0).toString(16).toUpperCase().padStart(8, '0')}` : String(code);
  let why = `The engine program ${program} ended with exit code ${shown} before it listened`;
  if (code !== null && code >>> 0 === LIBRARIES_MISSING) {
    why += ': the libraries of FFmpeg that it reads media with (avcodec, avformat, avutil, swscale) are neither beside it nor on the search path';
  }
  throw failure(`${why}.`);
}

/**
 * Start the program of the engine for a project, and wait until it says where it listens, or
 * that another engine has the project open. What it logs on stderr is appended to the log file
 * as it comes, and the last lines are kept to tell why it did not start.
 */
function start(options, rules) {
  const { program, project, env, logFile, kill } = options;
  return new Promise((resolve, reject) => {
    let settled = false;
    let timer = null;
    const settle = (act, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      act(value);
    };
    mkdirSync(path.dirname(logFile), { recursive: true });
    const log = createWriteStream(logFile, { flags: 'a' });
    log.on('error', () => {});
    let child;
    try {
      child = spawn(program, ['--project', project], {
        cwd: path.dirname(program), env, windowsHide: true,
        detached: rules.platform !== 'win32', stdio: ['pipe', 'pipe', 'pipe'],
      });
    } catch (error) {
      log.end();
      reject(failure(`The engine program ${program} could not be started: ${error.message}`));
      return;
    }
    const started = { child, ended: false, closed: null };
    started.closed = new Promise((ended) => {
      child.once('close', (code, signal) => {
        started.ended = true;
        log.end();
        ended({ code, signal });
      });
    });
    // What it logs is never left unread: the engine drops what it cannot write, and says so.
    const tail = [];
    const errors = new StringDecoder('utf8');
    let rest = '';
    child.stderr.on('data', (chunk) => {
      const parts = (rest + errors.write(chunk)).split('\n');
      rest = parts.pop().slice(0, TAIL_LINE_CHARS);
      for (const part of parts) {
        const line = part.trim();
        if (line) tail.push(line.slice(0, TAIL_LINE_CHARS));
      }
      if (tail.length > TAIL_LINES) tail.splice(0, tail.length - TAIL_LINES);
    });
    child.stderr.pipe(log);
    // An engine that ended before it read is told of by how it ended.
    child.stdin.on('error', () => {});
    const output = new StringDecoder('utf8');
    let first = '';
    let said = false;
    child.stdout.on('data', (chunk) => {
      // Nothing else is ever written there, and nothing is held up by what is not read.
      if (said) return;
      first += output.write(chunk);
      const end = first.indexOf('\n');
      if (end < 0) {
        if (first.length > FIRST_LINE_MOST) {
          said = true;
          kill(child);
          settle(reject, failure(`The engine program ${program} did not say where it listens.`));
        }
        return;
      }
      said = true;
      const line = first.slice(0, end).trim();
      let told = null;
      try { told = JSON.parse(line); } catch { /* No such line, as below. */ }
      const address = told?.listening;
      if (told === null || typeof told !== 'object' || told.protocol !== PROTOCOL || !engineAt(address, rules)) {
        kill(child);
        settle(reject, failure(`The engine did not say where it listens in protocol ${PROTOCOL}, at an address where an engine listens: ${line.slice(0, 200)}`));
        return;
      }
      settle(resolve, { kind: 'listening', address, pid: Number.isInteger(told.pid) ? told.pid : child.pid, started });
    });
    child.once('error', (error) => settle(reject, failure(`The engine program ${program} could not be started: ${error.message}`)));
    started.closed.then(({ code }) => {
      if (settled) return;
      const last = errors.end();
      if ((rest + last).trim()) tail.push((rest + last).trim().slice(0, TAIL_LINE_CHARS));
      try {
        const ended = endedWith(program, code, tail);
        settle(resolve, { ...ended, address: engineAt(ended.address, rules) ? ended.address : null });
      } catch (error) {
        settle(reject, error);
      }
    });
    timer = setTimeout(() => {
      kill(child);
      settle(reject, failure(`The engine program ${program} did not say where it listens within ${LISTENING_MS / 1000} seconds.`));
    }, LISTENING_MS);
  });
}

/**
 * Reach the engine at an address on two connections, and say hello on each. `started` is the
 * program that this process started, which it owns, or null for an engine that is somebody else's.
 */
async function sessionAt(socket, address, started, options, rules) {
  const { project, onEvent, kill } = options;
  let ready = false;
  let ending = null;
  let exit = { code: null, signal: null };
  let main = null;
  let view = null;
  const end = (expected) => {
    // Set before anything is closed: a connection that closes now calls this again.
    ending ??= Promise.resolve().then(async () => {
      main?.close();
      view?.close();
      if (started) exit = (await stopChild(started, kill)) ?? exit;
      onEvent({ kind: 'exited', expected, code: exit.code, signal: exit.signal });
    });
    return ending;
  };
  const gone = () => { if (ready) void end(false); };
  main = wireOf(socket, { notified: (method, params) => onEvent({ kind: 'notification', method, params }), gone });
  let hello;
  try {
    hello = await greet(main, project, rules.platform);
    const other = await connectTo(address);
    if (!other) throw failure(`The engine did not answer a second connection at ${address}.`);
    // Every connection that said hello is told of every change: the main one hands them on.
    view = wireOf(other, { notified: () => {}, gone });
    await greet(view, project, rules.platform);
  } catch (error) {
    main.close();
    view?.close();
    if (started) await stopChild(started, kill);
    throw error;
  }
  ready = true;
  if (started) {
    started.closed.then((ended) => {
      exit = ended;
      void end(false);
    });
  }
  return {
    owned: started !== null,
    pid: started ? started.pid : hello.pid,
    address,
    hello,
    get alive() { return ending === null; },
    /** Ask on the main connection. */
    call: (method, params) => main.call(method, params),
    /** Ask on the connection of the view. */
    view: (method, params) => view.call(method, params),
    /** Close the connections, and end the engine where it is this process's own. */
    close: () => end(true),
    /** Let go of everything at once, without waiting: the application ends. */
    abandon() {
      main.close();
      view.close();
      if (started) { try { started.child.stdin.end(); } catch { /* It has ended. */ } }
    },
  };
}

/**
 * Open the engine of a project: start one, or reach the one that has the project open already.
 *
 * Options: `program`, the engine; `project`, its directory; `env`, its environment; `logFile`,
 * where what it logs is appended; `kill`, how a program and what it started are ended; and
 * `onEvent`, which is told `{kind: 'notification', method, params}` for every notification, in
 * the order they came, and once `{kind: 'exited', expected, code, signal}` when the engine is
 * gone or was closed.
 *
 * Throws where no engine could be started or reached, with what the engine said where it said why.
 */
async function openEngine(options) {
  const rules = { platform: process.platform, env: options.env ?? {}, uid: typeof process.getuid === 'function' ? process.getuid() : 0 };
  let why = 'no engine answered';
  for (let tries = 0; tries < TRIES; tries += 1) {
    const started = await start(options, rules);
    if (started.kind === 'listening') {
      const socket = await connectTo(started.address);
      if (!socket) {
        await stopChild(started.started, options.kill);
        throw failure(`The engine that was started did not answer at ${started.address}.`);
      }
      started.started.pid = started.pid;
      return sessionAt(socket, started.address, started.started, options, rules);
    }
    // Another engine has the project open: an agent's may, without a window. It is reached where
    // it said it listens, or where `engine.lock` says so a moment later.
    let { address } = started;
    let socket = address === null ? null : await connectTo(address);
    if (!socket) {
      await moment();
      address = locked(options.project, rules);
      socket = address === null ? null : await connectTo(address);
    }
    if (socket) {
      try {
        return await sessionAt(socket, address, null, options, rules);
      } catch (error) {
        why = error.message;
        await moment();
        continue;
      }
    }
    why = started.kind === 'held' ? `the engine that has it open did not answer${address ? ` at ${address}` : ''}` : 'another engine is starting for it';
    await moment();
  }
  throw failure(`No engine for the project ${options.project} could be reached: ${why}.`);
}

module.exports = { openEngine, engineAt, PROTOCOL };
