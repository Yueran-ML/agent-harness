import { copyFile, mkdir, readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
let assetDirectory = process.env.COLUMBINA_ASSET_DIR;
if (!assetDirectory) {
  try {
    const local = JSON.parse(await readFile(path.join(desktop, 'assets.local.json'), 'utf8'));
    assetDirectory = local.assetDirectory;
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
}
if (typeof assetDirectory !== 'string' || !path.isAbsolute(assetDirectory)) {
  throw new Error('Set COLUMBINA_ASSET_DIR to an external artwork directory, or create assets.local.json as described in ASSETS.md.');
}

// Only these three visual assets enter the application. The original source clip
// and locally installed background music are never copied into public/.
const assets = ['columbina-intro-extended.mp4', 'moonlit-columbina.png', 'columbina-icon.png'];
for (const name of assets) {
  if (!(await stat(path.join(assetDirectory, name))).isFile()) {
    throw new Error(`Missing artwork file: ${name}. See ASSETS.md.`);
  }
}
await mkdir(path.join(desktop, 'public'), { recursive: true });
for (const name of assets) {
  await copyFile(path.join(assetDirectory, name), path.join(desktop, 'public', name));
}
await copyFile(path.join(desktop, 'ASSETS.md'), path.join(desktop, 'public', 'ASSET-NOTICES.txt'));
console.log(`Prepared ${assets.length} visual assets from ${assetDirectory}; no audio copied.`);
