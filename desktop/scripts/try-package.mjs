// Try the package that electron/package.mjs made on this system, as someone who downloads it would
// have it: the pages and the three visual files are in it, and its Python runs Columbina on its own.
// The Python is given a home of its own, a keyring that keeps nothing and a scripted model, so no key
// and no service of a model is needed. It answers a task, and it greets and leaves as the window has
// it do. Nothing of this is written into the package.
import { spawn, spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { checkPackageAssets } from './check-package-assets.mjs';

const systems = {
  win32: { arch: 'x64', python: ['python.exe'] },
  darwin: { arch: 'arm64', python: ['bin', 'python3'] },
};
const system = systems[process.platform];
if (!system || process.arch !== system.arch) throw new Error(`There is no package for ${process.platform} ${process.arch}.`);
const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const folder = path.join(desktop, 'release', `Columbina-${process.platform}-${system.arch}`);
const application = process.platform === 'darwin' ? path.join(folder, 'Columbina.app') : folder;
const resources = process.platform === 'darwin' ? path.join(application, 'Contents', 'Resources') : path.join(folder, 'resources');
const python = path.join(resources, 'python', ...system.python);
// What the shell starts the service with, before the command.
const flags = ['-I', '-B', '-m', 'columbina'];

/** Say what failed, and stop. */
function fail(why) {
  throw new Error(`The package is not as it should be: ${why}`);
}

/** Every file under a folder, with its size and the time it was last written. */
async function listing(root) {
  const files = new Map();
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const location = path.join(directory, entry.name);
      if (entry.isDirectory()) await visit(location);
      else {
        const details = await stat(location).catch(() => null);
        files.set(path.relative(root, location), details ? `${details.size} ${details.mtimeMs}` : 'a link to nothing');
      }
    }
  }
  await visit(root);
  return files;
}

/** What the Python of the package writes for a command of Columbina, once it has ended well. */
function said(args, options) {
  const result = spawnSync(python, [...flags, ...args], { encoding: 'utf8', timeout: 120_000, ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) fail(`columbina ${args[0]} ended with ${result.status ?? result.signal}:\n${result.stderr}`);
  return result.stdout.trim();
}

/** Greet the service and leave, as the window does, and return what it answered to the greeting. */
function greetAndLeave(options) {
  return new Promise((resolve, reject) => {
    const service = spawn(python, [...flags, 'serve'], { ...options, stdio: ['pipe', 'pipe', 'pipe'] });
    const answers = [];
    let errors = '';
    const timer = setTimeout(() => { service.kill(); reject(new Error('The service did not answer and leave within a minute.')); }, 60_000);
    service.stderr.on('data', (chunk) => { errors += chunk; });
    createInterface({ input: service.stdout }).on('line', (line) => {
      answers.push(JSON.parse(line));
      if (answers.length === 1) service.stdin.end(`${JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'shutdown' })}\n`);
    });
    service.on('error', reject);
    service.on('close', (code) => {
      clearTimeout(timer);
      if (code !== 0) reject(new Error(`The service ended with ${code}:\n${errors}`));
      else resolve(answers);
    });
    const greeting = { jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocol: 1, client: { name: 'try-package', version: '0' } } };
    service.stdin.write(`${JSON.stringify(greeting)}\n`);
  });
}

// What the window shows.
const page = path.join(resources, 'app', 'dist');
for (const name of ['index.html', 'key.html', 'feishu.html', 'columbina-intro-extended.mp4', 'moonlit-columbina.png', 'columbina-icon.png', 'ASSET-NOTICES.txt']) {
  await stat(path.join(page, name)).catch(() => fail(`it has no ${name}`));
}
await checkPackageAssets(page);

const before = await listing(path.join(resources, 'python'));
const work = await mkdtemp(path.join(tmpdir(), 'columbina-try-'));
let version;
try {
  const project = path.join(work, 'project');
  await mkdir(project);
  const env = { ...process.env, COLUMBINA_HOME: path.join(work, 'home'), PYTHON_KEYRING_BACKEND: 'keyring.backends.null.Keyring' };
  for (const name of Object.keys(env)) if (/KEY|TOKEN|SECRET/.test(name) && name !== 'PYTHON_KEYRING_BACKEND') delete env[name];

  version = said(['--version'], { env });
  if (!/^columbina \d+\.\d+\.\d+/.test(version)) fail(`it says its version is ${JSON.stringify(version)}`);

  // The assistant in Feishu: its command answers, and its Python has the package of its connection.
  const feishu = JSON.parse(said(['feishu', '--status', '--json'], { env }));
  if (feishu.set_up !== false || feishu.running !== false) fail(`the assistant in Feishu says ${JSON.stringify(feishu)}`);
  const connection = spawnSync(python, ['-I', '-B', '-c', 'import websockets'], { encoding: 'utf8', timeout: 120_000, env });
  if (connection.status !== 0) fail(`its Python has no websockets for the assistant in Feishu:\n${connection.stderr}`);

  const patch = path.join(work, 'patch.yml');
  await writeFile(patch, [
    '- id: model',
    '  plugin: columbina.llm.adapters.scripted',
    '  config:',
    '    provider: deepseek',
    '    replies:',
    '      - text: The package works.',
    '',
  ].join('\n'));
  const answer = said(['run', '--patch', patch, '--cwd', project, 'Say whether the package works.'], { env, cwd: project });
  if (answer !== 'The package works.') fail(`a task was answered with ${JSON.stringify(answer)}`);

  const [greeted, left, ...more] = await greetAndLeave({ env, cwd: project });
  // The folder is named as the service finds it, which may be another spelling of the same folder.
  const greetedIn = typeof greeted?.result?.cwd === 'string' ? path.basename(greeted.result.cwd) : null;
  if (greeted?.id !== 1 || greeted.result?.server?.name !== 'columbina' || greetedIn !== 'project') {
    fail(`the service greeted with ${JSON.stringify(greeted)}`);
  }
  if (left?.id !== 2 || 'error' in left || more.length > 0) fail(`the service left with ${JSON.stringify([left, ...more])}`);
} finally {
  await rm(work, { recursive: true, force: true });
}

const after = await listing(path.join(resources, 'python'));
const changed = [...new Set([...before.keys(), ...after.keys()])].filter((file) => before.get(file) !== after.get(file));
if (changed.length > 0) fail(`its Python wrote into it: ${changed.slice(0, 10).join(', ')}`);
if (process.platform === 'darwin') {
  const sealed = spawnSync('codesign', ['--verify', '--deep', '--strict', application], { encoding: 'utf8' });
  if (sealed.status !== 0) fail(`its seal is broken:\n${sealed.stderr}`);
}
console.log(`The package works: ${version}, ${before.size} files of Python, none written to.`);
