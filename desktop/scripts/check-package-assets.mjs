import { readdir } from 'node:fs/promises';
import path from 'node:path';

const audioExtensions = new Set(['.mp3', '.m4a', '.aac', '.ogg', '.oga', '.opus', '.flac', '.wav', '.aiff', '.wma']);
const videoExtensions = new Set(['.mp4', '.webm', '.mov', '.mkv', '.avi']);

export async function checkPackageAssets(dist) {
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const location = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        await visit(location);
      } else {
        const extension = path.extname(entry.name).toLowerCase();
        if (audioExtensions.has(extension)) {
          throw new Error(`Local background music must not enter a public package: ${path.relative(dist, location)}`);
        }
        if (videoExtensions.has(extension) && path.relative(dist, location) !== 'columbina-intro-extended.mp4') {
          throw new Error(`Only the silent extended opening belongs in the package: ${path.relative(dist, location)}`);
        }
      }
    }
  }
  await visit(dist);
}
