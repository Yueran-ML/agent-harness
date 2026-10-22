// The Python that a package of the application brings, with Columbina in it, put together in
// build/python. The packaging copies that folder into the resources of the application, and the
// shell starts the service from there.
//
// The interpreter is a CPython of python-build-standalone, which runs from whatever folder it lies
// in. uv fetches it at the version that is named here, so that every package has the same one.
// Columbina is built from this checkout, and what it needs is installed at the versions and with
// the hashes of uv.lock, the tools for servers of the Model Context Protocol and the connection of
// the assistant in Feishu among them. What a service never uses is taken out, and every module is
// compiled ahead, so that the package is not written to while it runs. Then the result is tried once.
import { spawnSync } from 'node:child_process';
import { cp, mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const PYTHON = '3.12.13';
const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const checkout = path.resolve(desktop, '..');
const target = path.join(desktop, 'build', 'python');
const work = path.join(desktop, 'build', 'python-work');
const windows = process.platform === 'win32';

/** Run a program to its end, and stop here if it does not end well. */
function run(program, args, options = {}) {
  console.log(`> ${[program, ...args].join(' ')}`);
  const result = spawnSync(program, args, { stdio: 'inherit', ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${program} ended with ${result.status ?? result.signal}.`);
}

/** What a program writes to its standard output, once it has ended well. */
function said(program, args, options = {}) {
  const result = spawnSync(program, args, { encoding: 'utf8', ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${program} ended with ${result.status ?? result.signal}: ${result.stderr}`);
  return result.stdout.trim();
}

/** The one entry of a folder whose name passes a test. */
async function onlyOne(folder, test, what) {
  const found = (await readdir(folder)).filter(test);
  if (found.length !== 1) throw new Error(`Expected one ${what} in ${folder}, and found ${found.length}.`);
  return found[0];
}

await rm(work, { recursive: true, force: true });
await rm(target, { recursive: true, force: true });

// The interpreter, in a folder of its own: the Pythons that uv keeps for the user are not touched.
// It is unpacked in the folder for temporary files, since on some drives of Windows uv cannot move
// what it has just unpacked, and then copied here, with its links as they are.
const interpreters = await mkdtemp(path.join(tmpdir(), 'columbina-python-'));
try {
  run('uv', ['python', 'install', PYTHON, '--install-dir', interpreters, '--no-bin']);
  const installed = await onlyOne(interpreters, (name) => name.startsWith(`cpython-${PYTHON}-`), 'interpreter');
  await cp(path.join(interpreters, installed), target, { recursive: true, verbatimSymlinks: true });
} finally {
  await rm(interpreters, { recursive: true, force: true });
}
const python = windows ? path.join(target, 'python.exe') : path.join(target, 'bin', 'python3');
const library = windows ? path.join(target, 'Lib') : path.join(target, 'lib', `python${PYTHON.split('.').slice(0, 2).join('.')}`);
// uv marks the Pythons that it keeps as its own to install into. This one belongs to the application.
await rm(path.join(library, 'EXTERNALLY-MANAGED'), { force: true });

// What Columbina needs, at the versions of the lock and checked against its hashes, and then Columbina.
const wheels = path.join(work, 'wheel');
const requirements = path.join(work, 'requirements.txt');
run('uv', ['build', '--wheel', '--out-dir', wheels], { cwd: checkout });
run('uv', [
  'export', '--quiet', '--locked', '--no-default-groups', '--extra', 'mcp', '--extra', 'feishu', '--extra', 'wecom', '--no-emit-project',
  '--format', 'requirements-txt', '--output-file', requirements,
], { cwd: checkout });
// Copied, and not linked to the cache of uv: the package is a folder of its own.
const install = ['pip', 'install', '--python', python, '--link-mode', 'copy', '--no-deps'];
run('uv', [...install, '--require-hashes', '--requirement', requirements]);
run('uv', [...install, path.join(wheels, await onlyOne(wheels, (name) => name.endsWith('.whl'), 'wheel'))]);

// What a service never uses: the windows and drawings of the standard library, pip, and what is
// there to build extensions. The programs of the packages go too: each calls the Python by the path
// that it was installed at, which is not where the package lies.
const removed = [
  ...['idlelib', 'tkinter', 'turtledemo'].map((name) => path.join(library, name)),
  ...(await readdir(path.join(library, 'site-packages'))).filter((name) => /^pip(-[\d.]+\.dist-info)?$/.test(name))
    .map((name) => path.join(library, 'site-packages', name)),
];
if (windows) {
  removed.push(...['Scripts', 'tcl', 'include', 'libs'].map((name) => path.join(target, name)));
  removed.push(...['_tkinter.pyd', 'tcl86t.dll', 'tk86t.dll'].map((name) => path.join(target, 'DLLs', name)));
} else {
  removed.push(...(await readdir(path.join(target, 'bin'))).filter((name) => !/^python3(\.\d+)?$/.test(name))
    .map((name) => path.join(target, 'bin', name)));
  removed.push(...['include', 'share'].map((name) => path.join(target, name)));
  // Only the scripts of Tcl and Tk: a library that the interpreter is linked against stays.
  removed.push(...(await readdir(path.join(target, 'lib'), { withFileTypes: true }))
    .filter((entry) => entry.isDirectory() && /^(tcl|tk)\d/.test(entry.name)).map((entry) => path.join(target, 'lib', entry.name)));
}
for (const location of removed) await rm(location, { recursive: true, force: true });
console.log(`Took out ${removed.map((location) => path.relative(target, location)).join(', ')}`);

// Every module compiled ahead, and its compiled form used without a look at the source: the files
// of a package keep the time at which it was unpacked, and a Python that compiled anew would write
// into the package, which on macOS breaks the seal of the application.
run(python, ['-I', '-m', 'compileall', '-q', '-f', '-j', '0', '--invalidation-mode', 'unchecked-hash', library]);

const version = said(python, ['-I', '-B', '-m', 'columbina', '--version']);
console.log(`Prepared ${version} on Python ${PYTHON} in ${target}`);
await rm(work, { recursive: true, force: true });
