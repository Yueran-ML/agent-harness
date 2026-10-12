import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { checkPackageAssets } from '../scripts/check-package-assets.mjs';

const directories: string[] = [];
async function bundle() {
  const directory = await mkdtemp(path.join(tmpdir(), 'columbina-package-'));
  directories.push(directory);
  return directory;
}
afterEach(async () => {
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

it('permits the runtime visual assets and their notice', async () => {
  const directory = await bundle();
  for (const name of ['columbina-intro-extended.mp4', 'moonlit-columbina.png', 'columbina-icon.png', 'ASSET-NOTICES.txt']) {
    await writeFile(path.join(directory, name), 'fixture');
  }
  await expect(checkPackageAssets(directory)).resolves.toBeUndefined();
});

it('rejects local BGM even in a nested asset directory', async () => {
  const directory = await bundle();
  await mkdir(path.join(directory, 'music'));
  await writeFile(path.join(directory, 'music', 'somnias-a-luna.m4a'), 'fixture');
  await expect(checkPackageAssets(directory)).rejects.toThrow('must not enter a public package');
});

it('rejects accidental inclusion of the original video', async () => {
  const directory = await bundle();
  await writeFile(path.join(directory, 'columbina-intro.mp4'), 'fixture');
  await expect(checkPackageAssets(directory)).rejects.toThrow('Only the silent extended opening');
});
