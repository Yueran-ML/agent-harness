// The notices of the Rust crates that are compiled into the engine of the studio, for the package:
// a table of each crate with its version and license, and the license files that each crate brings,
// copied into a folder of its own. The crates are those that `cargo metadata` resolves for the target
// from the package of the engine (columbina-server), following normal dependencies only: what is
// only built for the build (bindgen, cc) or only for the tests is not in the program, and neither
// is a procedural macro (serde_derive, thiserror-impl), which runs in the compiler: it and what only
// it depends on (syn, quote) are left out. The crates of the workspace are Columbina's own, under
// its MIT license, and are not listed.
//
//   node scripts/engine-notices.mjs --out <folder> [--engine <engine/>] [--target <triple>]
//
// writes <folder>/RUST-CRATES.md and <folder>/licenses/crates/<name>-<version>/. electron/package.mjs
// runs the same through rustCrates() and writeCrateNotices(). cargo is taken from CARGO, else PATH;
// it runs in the engine's folder, so that its rust-toolchain.toml holds.
import { spawnSync } from 'node:child_process';
import { copyFile, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

/** The package whose binary is the engine. */
const ENGINE_PACKAGE = 'columbina-server';

/** The target of a package made on this system. */
export function hostTarget(platform = process.platform, arch = process.arch) {
  const targets = {
    'win32 x64': 'x86_64-pc-windows-msvc',
    'darwin arm64': 'aarch64-apple-darwin',
    'darwin x64': 'x86_64-apple-darwin',
    'linux x64': 'x86_64-unknown-linux-gnu',
    'linux arm64': 'aarch64-unknown-linux-gnu',
  };
  const target = targets[`${platform} ${arch}`];
  if (!target) throw new Error(`There is no Rust target known for ${platform} ${arch}.`);
  return target;
}

/** Whether a file at the top of a crate is one of its licenses or notices. */
const isLicenseFile = (name) => /^(licen[cs]e|copying|notice|copyright|unlicense)/i.test(name);

/**
 * The third-party crates of the engine for a target: name, version, license expression, where its
 * source is, and the license files at the top of the crate (and in a LICENSES folder there).
 */
export async function rustCrates({ engine, target }) {
  const cargo = process.env.CARGO || 'cargo';
  const result = spawnSync(cargo, [
    'metadata', '--format-version', '1', '--locked', '--filter-platform', target,
    '--manifest-path', path.join(engine, 'Cargo.toml'),
  ], { cwd: engine, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024, shell: false });
  if (result.error) throw new Error(`cargo metadata could not be run (${cargo}): ${result.error.message}`);
  if (result.status !== 0) throw new Error(`cargo metadata ended with ${result.status}:\n${result.stderr}`);
  const metadata = JSON.parse(result.stdout);
  const packages = new Map(metadata.packages.map((one) => [one.id, one]));
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const members = new Set(metadata.workspace_members);
  const root = metadata.packages.find((one) => one.name === ENGINE_PACKAGE && members.has(one.id));
  if (!root) throw new Error(`The workspace of ${engine} has no package ${ENGINE_PACKAGE}.`);

  // Normal dependencies only, for the target that the metadata was filtered for, and no
  // procedural macro, nor what is reached only through one.
  const isMacro = (id) => packages.get(id).targets.some((target) => target.kind.includes('proc-macro'));
  const reached = new Set([root.id]);
  const waiting = [root.id];
  while (waiting.length > 0) {
    for (const dependency of nodes.get(waiting.pop()).deps) {
      if (!dependency.dep_kinds.some((kind) => kind.kind === null)) continue;
      if (reached.has(dependency.pkg) || isMacro(dependency.pkg)) continue;
      reached.add(dependency.pkg);
      waiting.push(dependency.pkg);
    }
  }

  const crates = [];
  for (const id of reached) {
    if (members.has(id)) continue;
    const one = packages.get(id);
    const folder = path.dirname(one.manifest_path);
    const files = [];
    for (const entry of await readdir(folder, { withFileTypes: true })) {
      if (entry.isFile() && isLicenseFile(entry.name)) files.push(entry.name);
      if (entry.isDirectory() && /^licen[cs]es$/i.test(entry.name)) {
        for (const inner of await readdir(path.join(folder, entry.name), { withFileTypes: true })) {
          if (inner.isFile()) files.push(`${entry.name}/${inner.name}`);
        }
      }
    }
    if (one.license_file && !files.includes(one.license_file.replaceAll('\\', '/'))) files.push(one.license_file.replaceAll('\\', '/'));
    crates.push({
      name: one.name, version: one.version, license: one.license ?? null, authors: one.authors ?? [],
      repository: one.repository ?? null, registry: (one.source ?? '').startsWith('registry+'),
      folder, files: files.sort(),
    });
  }
  crates.sort((one, other) => one.name.localeCompare(other.name) || one.version.localeCompare(other.version));
  return crates;
}

/**
 * The full text of a license, for a crate that brings none of its own: taken from another crate of
 * the same list that does (the Apache License 2.0 from any LICENSE-APACHE that is the license itself;
 * any other from a crate under that license alone, with a single license file).
 */
async function licenseText(spdx, crates) {
  for (const one of crates) {
    for (const file of one.files) {
      if (spdx === 'Apache-2.0' && !/apache/i.test(file)) continue;
      if (spdx !== 'Apache-2.0' && (one.license !== spdx || one.files.length !== 1)) continue;
      const text = await readFile(path.join(one.folder, file), 'utf8');
      if (spdx === 'Apache-2.0' && !/Version 2\.0, January 2004/.test(text)) continue;
      return { text, from: `${one.name} ${one.version}, ${file}` };
    }
  }
  return null;
}

/** Write the license files of the crates into <out>/licenses/crates, and answer the Markdown that lists them. */
export async function writeCrateNotices({ crates, out }) {
  const root = path.join(out, 'licenses', 'crates');
  await rm(root, { recursive: true, force: true });
  await mkdir(root, { recursive: true });
  const rows = [];
  for (const one of crates) {
    const named = `${one.name}-${one.version}`;
    const folder = path.join(root, named);
    await mkdir(folder, { recursive: true });
    let shipped = one.files;
    if (one.files.length > 0) {
      for (const file of one.files) {
        await mkdir(path.dirname(path.join(folder, file)), { recursive: true });
        await copyFile(path.join(one.folder, file), path.join(folder, file));
      }
    } else {
      // A crate published without a license file: the text of a license that its expression
      // offers, taken from another crate, with who wrote the crate.
      const offered = (one.license ?? '').split(/\s+OR\s+|\//).map((name) => name.replace(/[()]/g, '').trim());
      const chosen = offered.includes('Apache-2.0') ? 'Apache-2.0' : offered.length === 1 ? offered[0] : null;
      const found = chosen ? await licenseText(chosen, crates) : null;
      if (!found) throw new Error(`${one.name} ${one.version} brings no license file, and no text of ${one.license ?? 'its license'} was found among the other crates.`);
      const head = [
        `${one.name} ${one.version} (${one.repository ?? `https://crates.io/crates/${one.name}`})`,
        `Authors: ${one.authors.join(', ') || 'not named'}`,
        `License: ${one.license}${chosen !== one.license ? `, used here under ${chosen}` : ''}.`,
        `The crate as published has no license file. The text of ${chosen} below is that of ${found.from}.`,
        '', '',
      ].join('\n');
      await writeFile(path.join(folder, `LICENSE-${chosen}.txt`), head + found.text);
      shipped = [`LICENSE-${chosen}.txt`];
    }
    const source = one.repository ?? `https://crates.io/crates/${one.name}`;
    rows.push(`| ${one.name} | ${one.version} | ${one.license ?? 'not stated'} | ${source} | ${shipped.map((file) => `\`licenses/crates/${named}/${file}\``).join(', ')} |`);
  }
  return [
    `The engine is compiled from ${crates.length} crates of crates.io besides Columbina's own (MIT), and with`,
    'the standard library of Rust (MIT OR Apache-2.0, https://github.com/rust-lang/rust). Each crate\'s',
    'license files are copied unchanged into `licenses/crates/<name>-<version>/`.',
    '',
    '| Crate | Version | License | Source | License files |',
    '| --- | --- | --- | --- | --- |',
    ...rows,
    '',
  ].join('\n');
}

// Run by itself: write the notices of the crates into the folder that --out names.
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = new Map();
  for (let index = 2; index < process.argv.length; index += 2) args.set(process.argv[index], process.argv[index + 1]);
  const out = args.get('--out');
  if (!out) throw new Error('Name the folder to write into with --out.');
  const engine = path.resolve(args.get('--engine') ?? path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..', 'engine'));
  const crates = await rustCrates({ engine, target: args.get('--target') ?? hostTarget() });
  await mkdir(out, { recursive: true });
  await writeFile(path.join(out, 'RUST-CRATES.md'), `# Rust crates in the engine\n\n${await writeCrateNotices({ crates, out })}`);
  console.log(`${crates.length} crates, written to ${path.resolve(out)}`);
}
