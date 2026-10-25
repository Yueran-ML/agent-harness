import { copyFile, mkdir, readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { drawPets } from './draw-pets.mjs';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
// The artwork comes with the repository, in assets/. A directory of one's own may be put
// in its place, by COLUMBINA_ASSET_DIR or by a local, ignored assets.local.json.
let assetDirectory = process.env.COLUMBINA_ASSET_DIR;
if (!assetDirectory) {
  try {
    const local = JSON.parse(await readFile(path.join(desktop, 'assets.local.json'), 'utf8'));
    assetDirectory = local.assetDirectory;
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
}
assetDirectory ??= path.join(desktop, 'assets');
if (typeof assetDirectory !== 'string' || !path.isAbsolute(assetDirectory)) {
  throw new Error('COLUMBINA_ASSET_DIR and assets.local.json name the artwork directory by its absolute path, as described in ASSETS.md.');
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
// The pets that come with the application are drawn, and no picture of them is kept.
console.log(`Drew the pets ${drawPets(path.join(desktop, 'public', 'pets')).join(', ')}.`);
