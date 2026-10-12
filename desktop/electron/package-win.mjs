import { packager } from '@electron/packager';
import { readFile, realpath, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkPackageAssets } from '../scripts/check-package-assets.mjs';

const desktop = await realpath(path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..'));
const output = path.join(desktop, 'release');
const target = path.join(output, 'Columbina-win32-x64');
// Packager may replace a previous build. Reject a symlink or junction that would
// redirect that replacement outside the explicitly named release directory.
for (const directory of [output, target]) {
  try {
    if ((await realpath(directory)).toLowerCase() !== directory.toLowerCase()) throw new Error(`The release path must not be redirected: ${directory}`);
  } catch (error) { if (error.code !== 'ENOENT') throw error; }
}
await stat(path.join(desktop, 'dist', 'index.html'));
await checkPackageAssets(path.join(desktop, 'dist'));
const manifest = JSON.parse(await readFile(path.join(desktop, 'package.json'), 'utf8'));
const electron = JSON.parse(await readFile(path.join(desktop, 'node_modules/electron/package.json'), 'utf8'));
const runtimeFiles = new Set([
  '/package.json', '/electron', '/electron/main.cjs', '/electron/preload.cjs',
  '/electron/mock-service.mjs', '/electron/icon.ico', '/src',
  '/src/replay.shared.js', '/src/mock-fixture.js',
]);
console.log(`Packaging Windows application in ${target}`);
const packages = await packager({
  dir: desktop, out: output, name: 'Columbina', executableName: 'Columbina',
  platform: 'win32', arch: 'x64', electronVersion: electron.version,
  appVersion: manifest.version, buildVersion: manifest.version,
  icon: path.join(desktop, 'electron', 'icon.ico'),
  appBundleId: 'dev.columbina.desktop',
  win32metadata: { CompanyName: 'Columbina', FileDescription: 'Columbina agent workspace', ProductName: 'Columbina' },
  asar: false, prune: false, overwrite: true,
  ignore: (entry) => entry !== '' && entry !== '/dist' && !entry.startsWith('/dist/') && !runtimeFiles.has(entry),
});
for (const location of packages) console.log(`Ready to open: ${path.join(location, 'Columbina.exe')}`);
