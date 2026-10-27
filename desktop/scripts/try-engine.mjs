// Try the engine of the studio in the package that electron/package.mjs made on Windows, as on a
// system with nothing else installed: with Windows alone on PATH, and no FFMPEG_DIR,
// COLUMBINA_ENGINE or COLUMBINA_PROXY_FFMPEG.
//   - resources/engine holds exactly the engine, the libraries of FFmpeg, its ffmpeg and the notices;
//   - the engine needs no C runtime beside it (it is linked with it statically);
//   - each library says that it is of the LGPL, and that ffmpeg starts and is an LGPL build;
//   - a headless engine starts with the libraries beside it, says where it listens, and ends by
//     itself once it has been idle;
//   - an engine of the desktop's kind takes in a film of 4K HEVC of 10 bits, finds the ffmpeg
//     beside it, and makes the film's proxy with it.
// The film is made by an ffmpeg with libx265: COLUMBINA_TEST_FFMPEG by its whole path, else the
// first on PATH (on CI the GPL build of .github/scripts/ffmpeg.sh, named by COLUMBINA_TEST_FFMPEG;
// it is not in the package). Nothing is written into the package but what the engine writes in a project of its own.
//
//   node scripts/try-engine.mjs
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { existsSync } from 'node:fs';
import { mkdtemp, readdir, readFile, rm } from 'node:fs/promises';
import net from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { cleanEnvironment, lgplConfiguration, libraryLicense, runtimeImports } from '../electron/engine-bundle.mjs';

if (process.platform !== 'win32') {
  console.log('Packages bring an engine only on Windows, for now: there is nothing to try here.');
  process.exit(0);
}
const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const folder = path.join(desktop, 'release', `Columbina-${process.platform}-${process.arch}`, 'resources', 'engine');
const engine = path.join(folder, 'columbina-engine.exe');

/** Say what failed, and stop. */
function fail(why) {
  throw new Error(`The engine of the package is not as it should be: ${why}`);
}

// What the folder holds: no more, no less.
const expected = [
  'THIRD_PARTY_ENGINE.md', 'avcodec-62.dll', 'avdevice-62.dll', 'avfilter-11.dll', 'avformat-62.dll', 'avutil-60.dll',
  'columbina-engine.exe', 'ffmpeg.exe', 'licenses', 'swresample-6.dll', 'swscale-9.dll',
];
const found = (await readdir(folder).catch(() => fail(`there is no ${folder}`))).sort();
if (JSON.stringify(found) !== JSON.stringify(expected)) fail(`it holds ${found.join(', ')}, and not ${expected.join(', ')}`);
for (const notice of ['licenses/FFmpeg/LICENSE.txt', 'licenses/FFmpeg/GPL-3.0.txt']) {
  if (!existsSync(path.join(folder, notice))) fail(`it has no ${notice}`);
}
const crates = await readdir(path.join(folder, 'licenses', 'crates'));
const listed = (await readFile(path.join(folder, 'THIRD_PARTY_ENGINE.md'), 'utf8')).match(/^\| \S+ \| \d\S* \|/gm) ?? [];
if (crates.length === 0 || listed.length !== crates.length) fail(`THIRD_PARTY_ENGINE.md lists ${listed.length} crates, and licenses/crates holds ${crates.length}`);

const runtime = runtimeImports(engine);
if (runtime.length > 0) fail(`the engine imports ${runtime.join(', ')}, and is to have the C runtime in itself`);
// The libraries, and the ffmpeg of the package with nothing but Windows on PATH, are of the LGPL.
for (const name of found.filter((one) => one.endsWith('.dll'))) libraryLicense(path.join(folder, name));
lgplConfiguration(path.join(folder, 'ffmpeg.exe'));
const env = cleanEnvironment();
for (const name of Object.keys(env)) if (/^(FFMPEG_DIR|COLUMBINA_)/i.test(name)) delete env[name];

/** The first line that a program writes on stdout, as JSON, within a time. */
async function firstLine(child, seconds) {
  const lines = createInterface({ input: child.stdout });
  const timer = setTimeout(() => child.kill(), seconds * 1000);
  try {
    const [line] = await Promise.race([
      once(lines, 'line'),
      once(child, 'exit').then(([code]) => fail(`the engine ended with ${code} (${code >>> 0 === 0xc0000135 ? 'a library was not found' : 'see its stderr'}) before it listened`)),
    ]);
    return JSON.parse(line);
  } finally {
    clearTimeout(timer);
  }
}

/** The stderr of the engine, one JSON object a line, kept. */
function logOf(child) {
  const events = [];
  createInterface({ input: child.stderr }).on('line', (line) => {
    try { events.push(JSON.parse(line)); } catch { events.push({ text: line }); }
  });
  return events;
}

const work = await mkdtemp(path.join(tmpdir(), 'columbina-try-engine-'));
try {
  // A headless engine: it listens, and ends with 0 once it has been idle for 2 s.
  const headless = spawn(engine, ['--project', path.join(work, 'headless'), '--headless', '--idle-seconds', '2'], { env, cwd: work, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
  const headlessLog = logOf(headless);
  const said = await firstLine(headless, 30);
  if (typeof said.listening !== 'string' || said.protocol !== 1) fail(`the headless engine said ${JSON.stringify(said)}`);
  const ending = setTimeout(() => headless.kill(), 30_000);
  const [code] = await once(headless, 'exit');
  clearTimeout(ending);
  if (code !== 0) fail(`the headless engine ended with ${code} after it was idle:\n${JSON.stringify(headlessLog)}`);
  console.log(`A headless engine listened at ${said.listening} and ended by itself once idle.`);

  // A film of 4K HEVC of 10 bits, in long groups of frames: one that is given a proxy.
  const maker = process.env.COLUMBINA_TEST_FFMPEG || 'ffmpeg';
  const film = path.join(work, 'film-4k-hevc10.mp4');
  const made = spawnSync(maker, [
    '-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=size=3840x2160:rate=30:duration=2',
    '-c:v', 'libx265', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p10le', '-x265-params', 'log-level=error:keyint=30', '-tag:v', 'hvc1', film,
  ], { encoding: 'utf8', timeout: 300_000 });
  if (made.error || made.status !== 0) fail(`the film could not be made with ${maker} (an ffmpeg with libx265, named by COLUMBINA_TEST_FFMPEG):\n${made.error?.message ?? made.stderr}`);

  // The engine of the desktop: it ends when its stdin is closed, and makes proxies.
  const project = path.join(work, 'studio');
  const desktopEngine = spawn(engine, ['--project', project], { env, cwd: work, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
  const log = logOf(desktopEngine);
  try {
    const listening = await firstLine(desktopEngine, 30);
    const connection = net.connect(listening.listening);
    await once(connection, 'connect');
    const answers = new Map();
    const proxies = [];
    createInterface({ input: connection }).on('line', (line) => {
      const message = JSON.parse(line);
      if (message.id !== undefined) answers.set(message.id, message);
      else if (message.method === 'media.proxy') proxies.push(message.params);
    });
    let id = 0;
    const ask = async (method, params) => {
      id += 1;
      const asked = id;
      connection.write(`${JSON.stringify({ jsonrpc: '2.0', id: asked, method, params })}\n`);
      for (let waited = 0; !answers.has(asked); waited += 50) {
        if (waited > 60_000) fail(`${method} was not answered within a minute`);
        await new Promise((resolve) => setTimeout(resolve, 50));
      }
      const answer = answers.get(asked);
      if (answer.error) fail(`${method} was answered with ${JSON.stringify(answer.error)}`);
      return answer.result;
    };
    const hello = await ask('hello', { client: 'cli', protocol: 1 });
    if (hello.headless !== false) fail(`the engine of the desktop said hello with ${JSON.stringify(hello)}`);
    const { added: [media] } = await ask('media.import', { paths: [film] });
    const started = Date.now();
    let last;
    while (!last || !['done', 'failed'].includes(last.state)) {
      if (Date.now() - started > 300_000) fail(`the proxy was not made within 5 minutes: ${JSON.stringify(proxies)}`);
      await new Promise((resolve) => setTimeout(resolve, 200));
      last = proxies.filter((one) => one.media === media).at(-1);
    }
    const ffmpegFound = log.find((event) => event.event === 'proxy_ffmpeg');
    if (!ffmpegFound?.path || path.resolve(ffmpegFound.path).toLowerCase() !== path.join(folder, 'ffmpeg.exe').toLowerCase()) {
      fail(`the engine took the ffmpeg of its proxies from ${JSON.stringify(ffmpegFound)}, not from beside it`);
    }
    if (last.state !== 'done') fail(`the proxy failed: ${last.message}\n${JSON.stringify(log.filter((event) => /^proxy_/.test(event.event ?? '')))}`);
    const proxyMade = log.find((event) => event.event === 'proxy_made');
    const files = await readdir(path.join(project, 'cache', 'proxies'));
    if (!files.some((name) => name.startsWith(`${media}-`) && name.endsWith('.mov'))) fail(`there is no proxy in cache/proxies: ${files.join(', ')}`);
    console.log(`The engine of the desktop made the proxy of 4K HEVC of 10 bits with ${ffmpegFound.path} (${JSON.stringify(proxyMade ?? {})}).`);
    connection.destroy();
  } finally {
    desktopEngine.stdin.end();
    const timer = setTimeout(() => desktopEngine.kill(), 30_000);
    const [ended] = desktopEngine.exitCode === null ? await once(desktopEngine, 'exit') : [desktopEngine.exitCode];
    clearTimeout(timer);
    if (ended !== 0) { console.error(`The engine of the desktop ended with ${ended} once its stdin was closed.`); process.exitCode = 1; }
  }
} finally {
  await rm(work, { recursive: true, force: true });
}
console.log('The engine of the package works on its own.');
