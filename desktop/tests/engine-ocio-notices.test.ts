import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, expect, it } from 'vitest';
import { locateOcio, OCIO_LICENSES, OCIO_PARTS, OCIO_VERSION, writeOcioNotices } from '../scripts/engine-notices.mjs';

const desktop = fileURLToPath(new URL('..', import.meta.url));
const repository = path.resolve(desktop, '..');

const directories: string[] = [];
async function folder() {
  const directory = await mkdtemp(path.join(tmpdir(), 'columbina-ocio-'));
  directories.push(directory);
  return directory;
}
afterEach(async () => {
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

/** An install of OpenColorIO as build-ocio.ps1 leaves it: its version, and the static libraries in lib/. */
async function install(version: string, libraries = ['OpenColorIO.lib', 'yaml-cpp.lib', 'libexpatMT.lib', 'Imath-3_2.lib', 'pystring.lib', 'minizip-ng.lib', 'zs.lib', 'zlibstatic.lib']) {
  const directory = await folder();
  await mkdir(path.join(directory, 'lib', 'cmake', 'OpenColorIO'), { recursive: true });
  await writeFile(path.join(directory, 'lib', 'cmake', 'OpenColorIO', 'OpenColorIOConfigVersion.cmake'), `set(PACKAGE_VERSION "${version}")\n`);
  for (const name of libraries) await writeFile(path.join(directory, 'lib', name), '');
  return directory;
}

it('has a notice for each library that the crate of OpenColorIO links, and for no other', async () => {
  const build = await readFile(path.join(repository, 'engine', 'crates', 'ocio', 'build.rs'), 'utf8');
  const linked = [...build.matchAll(/^\s*\("([^"]+)", &\[/gm)].map(match => match[1]);
  expect(linked.length).toBeGreaterThan(0);
  expect(OCIO_PARTS.map(part => part.library).sort()).toEqual([...linked].sort());
});

it('brings the license files of each part, and nothing that no part names', async () => {
  const named = new Set<string>();
  for (const part of OCIO_PARTS) {
    expect(part.files.length).toBeGreaterThan(0);
    for (const file of part.files) {
      const text = await readFile(path.join(OCIO_LICENSES, part.name, file), 'utf8');
      expect(text.trim().length).toBeGreaterThan(100);
      named.add(`${part.name}/${file}`);
    }
  }
  const present: string[] = [];
  for (const entry of await readdir(OCIO_LICENSES, { withFileTypes: true })) {
    expect(entry.isDirectory()).toBe(true);
    for (const file of await readdir(path.join(OCIO_LICENSES, entry.name))) present.push(`${entry.name}/${file}`);
  }
  expect(present.sort()).toEqual([...named].sort());
});

it('is of the version of OpenColorIO that CI and the recipe build', async () => {
  const recipe = await readFile(path.join(repository, 'engine', 'crates', 'ocio', 'ci', 'build-ocio.ps1'), 'utf8');
  expect(/\[string\] \$Version = '([^']+)'/.exec(recipe)?.[1]).toBe(OCIO_VERSION);
  for (const workflow of ['ci.yml', 'desktop.yml']) {
    const text = await readFile(path.join(repository, '.github', 'workflows', workflow), 'utf8');
    expect(/OCIO_VERSION: "([^"]+)"/.exec(text)?.[1], workflow).toBe(OCIO_VERSION);
  }
  expect(OCIO_PARTS.find(part => part.name === 'OpenColorIO')?.version).toBe(OCIO_VERSION);
});

it('tells of OpenColorIO, its built-in ACES configs and each library linked with it in THIRD_PARTY_ENGINE.md', async () => {
  const notice = await readFile(path.join(desktop, 'THIRD_PARTY_ENGINE.md'), 'utf8');
  expect(notice).toMatch(/^## OpenColorIO$/m);
  expect(notice).toMatch(/ACES/);
  for (const part of OCIO_PARTS) expect(notice, part.name).toContain(part.name);
});

it('copies the license files into licenses/<part>/ and lists each part with its version', async () => {
  const out = await folder();
  const list = await writeOcioNotices({ out });
  for (const part of OCIO_PARTS) {
    for (const file of part.files) {
      expect(existsSync(path.join(out, 'licenses', part.name, file)), `${part.name}/${file}`).toBe(true);
      expect(list).toContain(`\`licenses/${part.name}/${file}\``);
    }
    expect(list).toMatch(new RegExp(`^\\| ${part.name} \\| ${part.version.replaceAll('.', '\\.')} \\| ${part.license} \\|`, 'm'));
  }
});

it('takes the OpenColorIO of OCIO_DIR or --ocio=, of the version of the notices, with every library', async () => {
  const good = await install(OCIO_VERSION);
  expect(locateOcio({ args: [], env: { OCIO_DIR: good } })).toEqual({ folder: good, version: OCIO_VERSION });
  expect(locateOcio({ args: [`--ocio=${good}`], env: {} }).folder).toBe(good);

  expect(() => locateOcio({ args: [], env: {} })).toThrow(/OCIO_DIR/);
  const older = await install('2.5.1');
  expect(() => locateOcio({ args: [], env: { OCIO_DIR: older } })).toThrow(/2\.5\.1.*2\.6\.0|2\.6\.0.*2\.5\.1/);
  const lacking = await install(OCIO_VERSION, ['OpenColorIO.lib', 'yaml-cpp.lib', 'libexpatMT.lib', 'Imath-3_2.lib', 'pystring.lib', 'minizip-ng.lib']);
  expect(() => locateOcio({ args: [], env: { OCIO_DIR: lacking } })).toThrow(/zlib/);
  const empty = await folder();
  expect(() => locateOcio({ args: [], env: { OCIO_DIR: empty } })).toThrow(/OpenColorIOConfigVersion\.cmake/);
});
