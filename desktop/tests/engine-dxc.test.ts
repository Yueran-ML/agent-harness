import { existsSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { compareVersions, DXC_FILES, dxcIn, locateDxc, peFileVersion } from '../electron/engine-bundle.mjs';

const directories: string[] = [];
async function folder(files: Record<string, Buffer | string> = {}) {
  const directory = await mkdtemp(path.join(tmpdir(), 'columbina-dxc-'));
  directories.push(directory);
  for (const [name, content] of Object.entries(files)) await writeFile(path.join(directory, name), content);
  return directory;
}
afterEach(async () => {
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

/** The start of a PE file, and a VS_FIXEDFILEINFO of this version somewhere after it. */
function versioned([a, b, c, d]: number[]) {
  const buffer = Buffer.alloc(0x200);
  buffer.write('MZ', 0, 'latin1');
  const at = 0x120;
  buffer.writeUInt32LE(0xfeef04bd, at);
  buffer.writeUInt32LE(0x00010000, at + 4);
  buffer.writeUInt32LE(((a << 16) | b) >>> 0, at + 8);
  buffer.writeUInt32LE(((c << 16) | d) >>> 0, at + 12);
  return buffer;
}

it('reads the file version of a PE file', () => {
  expect(peFileVersion(versioned([1, 8, 2502, 11]))).toEqual([1, 8, 2502, 11]);
  expect(peFileVersion(versioned([101, 7, 2308, 24]))).toEqual([101, 7, 2308, 24]);
  expect(peFileVersion(Buffer.concat([Buffer.from('MZ'), Buffer.alloc(0x100)]))).toBeUndefined();
  expect(() => peFileVersion(Buffer.alloc(0x100), 'x.dll')).toThrow(/x\.dll is not a PE file/);
});

it('compares versions part by part', () => {
  expect(compareVersions([1, 8, 2502, 11], [1, 8, 2502, 0])).toBeGreaterThan(0);
  expect(compareVersions([1, 8, 2407, 12], [1, 8, 2502, 0])).toBeLessThan(0);
  expect(compareVersions([1, 9, 0, 0], [1, 8, 2502, 0])).toBeGreaterThan(0);
  expect(compareVersions([1, 8, 2502, 0], [1, 8, 2502])).toBe(0);
});

it('takes no DXC that is missing a file', async () => {
  const alone = await folder({ 'dxcompiler.dll': versioned([1, 8, 2502, 11]) });
  expect(dxcIn(alone)).toEqual({ folder: alone, why: 'there is no dxil.dll' });
});

it('packages without DXC only when asked to, and else says what to do', async () => {
  expect(locateDxc({ args: ['--dxc=none'], env: {} })).toHaveProperty('skipped');
  expect(locateDxc({ args: [], env: { COLUMBINA_DXC: 'NONE' } })).toHaveProperty('skipped');
  const empty = await folder();
  expect(() => locateDxc({ args: [`--dxc=${empty}`], env: {} })).toThrow(/no DXC of 1\.8\.2502 or later.*there is no dxcompiler\.dll.*--dxc=none/s);
});

const sdk = 'C:\\Program Files (x86)\\Windows Kits\\10\\Redist\\D3D\\x64';
it.skipIf(process.platform !== 'win32' || !existsSync(path.join(sdk, 'dxil.dll')))('finds the DXC of the Windows SDK installed, both files of one version', () => {
  const found = locateDxc({ args: [], env: { 'ProgramFiles(x86)': 'C:\\Program Files (x86)' } });
  if (!('files' in found) || !found.files || !found.version) throw new Error(`no DXC: ${JSON.stringify(found)}`);
  expect(found.files.map(([name]) => name)).toEqual(DXC_FILES);
  expect(compareVersions(found.version.split('.').map(Number), [1, 8, 2502, 0])).toBeGreaterThanOrEqual(0);
});
