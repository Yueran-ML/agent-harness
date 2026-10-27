// Package the application for the system that this runs on, and zip it for a release: Windows on
// x64, or macOS on Apple silicon. The package brings the Python of build/python, which runs on the
// system that it was put together on, so each package is made on its own system.
//
// Before this: `npm run build`, and `node scripts/prepare-python.mjs`. `npm run package` does all three.
import { packager } from '@electron/packager';
import { spawnSync } from 'node:child_process';
import { cp, readdir, readFile, realpath, rm, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkPackageAssets } from '../scripts/check-package-assets.mjs';

const systems = {
  win32: { arch: 'x64', named: 'windows-x64', icon: 'icon.ico', python: ['python.exe'] },
  darwin: { arch: 'arm64', named: 'macos-arm64', icon: 'icon.icns', python: ['bin', 'python3'] },
};
const system = systems[process.platform];
if (!system || process.arch !== system.arch) {
  throw new Error(`Packages are made on Windows x64 and on macOS on Apple silicon, and this is ${process.platform} ${process.arch}.`);
}

const desktop = await realpath(path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..'));
const output = path.join(desktop, 'release');
const folder = path.join(output, `Columbina-${process.platform}-${system.arch}`);
// Packager may replace a previous build. Reject a symlink or junction that would
// redirect that replacement outside the explicitly named release directory.
for (const directory of [output, folder]) {
  try {
    if ((await realpath(directory)).toLowerCase() !== directory.toLowerCase()) throw new Error(`The release path must not be redirected: ${directory}`);
  } catch (error) { if (error.code !== 'ENOENT') throw error; }
}
await stat(path.join(desktop, 'dist', 'index.html'));
await checkPackageAssets(path.join(desktop, 'dist'));
const python = path.join(desktop, 'build', 'python');
try {
  await stat(path.join(python, ...system.python));
} catch {
  throw new Error('There is no Python to bring along in build/python. Run node scripts/prepare-python.mjs first.');
}

/** Run a program to its end, and stop here if it does not end well. */
function run(program, args, options = {}) {
  const result = spawnSync(program, args, { stdio: 'inherit', ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${program} ended with ${result.status ?? result.signal}.`);
}

// The icon of the system, made from the dove.
if (process.platform === 'win32') {
  run('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', path.join(desktop, 'electron', 'create-icon.ps1')]);
} else {
  run('bash', [path.join(desktop, 'electron', 'create-icon.sh')]);
}

const manifest = JSON.parse(await readFile(path.join(desktop, 'package.json'), 'utf8'));
const electron = JSON.parse(await readFile(path.join(desktop, 'node_modules/electron/package.json'), 'utf8'));
const runtimeFiles = new Set([
  '/package.json', '/electron', '/electron/main.cjs', '/electron/preload.cjs', '/electron/key-preload.cjs', '/electron/feishu-preload.cjs',
  '/electron/wecom-preload.cjs', '/electron/qq-preload.cjs', '/electron/pet-preload.cjs', '/electron/pets.cjs', '/electron/engine.cjs',
  '/electron/mock-service.mjs', '/electron/icon.ico', '/src',
  '/src/replay.shared.js', '/src/mock-fixture.js',
]);
console.log(`Packaging the application in ${folder}`);
await packager({
  dir: desktop, out: output, name: 'Columbina', executableName: 'Columbina',
  platform: process.platform, arch: system.arch, electronVersion: electron.version,
  appVersion: manifest.version, buildVersion: manifest.version,
  icon: path.join(desktop, 'electron', system.icon),
  appBundleId: 'dev.columbina.desktop', appCategoryType: 'public.app-category.developer-tools',
  win32metadata: { CompanyName: 'Columbina', FileDescription: 'Columbina agent workspace', ProductName: 'Columbina' },
  asar: false, prune: false, overwrite: true,
  ignore: (entry) => entry !== '' && entry !== '/dist' && !entry.startsWith('/dist/') && !runtimeFiles.has(entry),
});

// The Python goes in after the packager, which would make every link in it an absolute one that
// points into this checkout. Here the links stay as they are.
const application = process.platform === 'darwin' ? path.join(folder, 'Columbina.app') : folder;
const resources = process.platform === 'darwin' ? path.join(application, 'Contents', 'Resources') : path.join(folder, 'resources');
await cp(python, path.join(resources, 'python'), { recursive: true, verbatimSymlinks: true });

const zipped = path.join(output, `Columbina-${manifest.version}-${system.named}.zip`);
await rm(zipped, { force: true });
if (process.platform === 'darwin') {
  // The packager changed what Electron had signed. A signature of its own, made without a
  // certificate, lets Apple silicon start it; the seal covers the Python as well.
  run('codesign', ['--force', '--deep', '--sign', '-', application]);
  run('codesign', ['--verify', '--deep', '--strict', '--verbose=2', application]);
  // ditto keeps the links of the frameworks, and what macOS keeps beside a file.
  run('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', application, zipped]);
} else {
  // The tar of Windows writes a zip. What the folder holds lies at the top of it, so that the folder
  // that Windows unpacks it into is named after the zip, as the folder of the application.
  const tar = path.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe');
  run(tar, ['-a', '-c', '-f', zipped, ...(await readdir(folder)).sort()], { cwd: folder });
}
console.log(`Ready to open: ${process.platform === 'darwin' ? application : path.join(folder, 'Columbina.exe')}`);
console.log(`Ready to publish: ${zipped}`);
