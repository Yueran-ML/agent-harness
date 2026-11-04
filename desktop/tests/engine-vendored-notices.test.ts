import { existsSync } from 'node:fs';
import { mkdtemp, readdir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, expect, it } from 'vitest';
import { VENDORED_LICENSES, VENDORED_PARTS, writeVendoredNotices } from '../scripts/engine-notices.mjs';

const desktop = fileURLToPath(new URL('..', import.meta.url));
const repository = path.resolve(desktop, '..');

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

it('has a notice for each library vendored into the crate of the stretch, and for no other', async () => {
  expect(VENDORED_LICENSES).toBe(path.join(repository, 'engine', 'crates', 'stretch', 'vendor'));
  const folders = (await readdir(VENDORED_LICENSES, { withFileTypes: true }))
    .filter(entry => entry.isDirectory())
    .map(entry => entry.name);
  expect(VENDORED_PARTS.map(part => part.name).sort()).toEqual(folders.sort());
});

it('brings the license file of each, an MIT license of its own', async () => {
  for (const part of VENDORED_PARTS) {
    expect(part.files.length).toBeGreaterThan(0);
    for (const file of part.files) {
      const text = await readFile(path.join(VENDORED_LICENSES, part.name, file), 'utf8');
      expect(text, `${part.name}/${file}`).toMatch(/^MIT License/);
      expect(text).toMatch(/Permission is hereby granted, free of charge/);
    }
  }
});

it('is of the versions and commits that the crate of the stretch says it vendors', async () => {
  const crate = await readFile(path.join(repository, 'engine', 'crates', 'stretch', 'src', 'lib.rs'), 'utf8');
  for (const part of VENDORED_PARTS) {
    const commit = /\(([0-9a-f]{40})\)$/.exec(part.source)?.[1];
    expect(commit, part.name).toBeDefined();
    const row = crate.split('\n').find(line => line.includes(`\`${commit}\``));
    expect(row, `the row of ${part.name} in lib.rs`).toBeDefined();
    expect(row).toContain(`tag \`${part.version}\``);
    expect(part.source).toContain(`tag ${part.version} `);
  }
});

it('tells of Signalsmith Stretch and Linear in THIRD_PARTY_ENGINE.md', async () => {
  const notice = await readFile(path.join(desktop, 'THIRD_PARTY_ENGINE.md'), 'utf8');
  expect(notice).toMatch(/^## Signalsmith Stretch$/m);
  for (const part of VENDORED_PARTS) {
    expect(notice, part.title).toContain(part.title);
    for (const file of part.files) expect(notice).toContain(`licenses/${part.name}/${file}`);
  }
});

it('copies the license files into licenses/<name>/ and lists each with its version', async () => {
  const out = await mkdtemp(path.join(tmpdir(), 'columbina-vendored-'));
  directories.push(out);
  const list = await writeVendoredNotices({ out });
  for (const part of VENDORED_PARTS) {
    expect(list).toContain(`| ${part.title} | ${part.version} | ${part.license} |`);
    for (const file of part.files) {
      const copied = path.join(out, 'licenses', part.name, file);
      expect(existsSync(copied), copied).toBe(true);
      expect(await readFile(copied, 'utf8')).toBe(await readFile(path.join(VENDORED_LICENSES, part.name, file), 'utf8'));
      expect(list).toContain(`\`licenses/${part.name}/${file}\``);
    }
  }
});
