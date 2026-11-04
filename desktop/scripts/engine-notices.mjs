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
// writes <folder>/RUST-CRATES.md and <folder>/licenses/crates/<name>-<version>/ (a few crates as
// OVERRIDES below says: one published without a license file, from crate-licenses/; one under a
// choice of licenses, with the files of the one it is used under). electron/package.mjs
// runs the same through rustCrates() and writeCrateNotices(). cargo is taken from CARGO, else PATH;
// it runs in the engine's folder, so that its rust-toolchain.toml holds.
//
// The notices of OpenColorIO, which the engine links statically with the C++ libraries that it is
// built with, are OCIO_PARTS below and the license files of licenses/ocio/: electron/package.mjs
// checks with locateOcio() that the OpenColorIO of OCIO_DIR is of their version, and writes them with
// writeOcioNotices(). tests/engine-ocio-notices.test.ts holds them to engine/crates/ocio/build.rs.
//
// The notices of the C++ libraries vendored into the engine's crates (Signalsmith Stretch and Linear,
// engine/crates/stretch/vendor/) are VENDORED_PARTS below, their license files read from those
// folders; electron/package.mjs writes them with writeVendoredNotices().
import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { copyFile, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

/** The package whose binary is the engine. */
const ENGINE_PACKAGE = 'columbina-server';

/** The folder of the license files that are shipped in place of those that a crate publishes. */
const CRATE_LICENSES = path.join(path.dirname(fileURLToPath(import.meta.url)), 'crate-licenses');

/**
 * Crates whose license files are not shipped as they are published:
 * - `files`: files of crate-licenses/<name>/, shipped in place of the crate's, for a crate that is
 *   published without any; `source` says where they were taken from.
 * - `only` and `under`: of a crate under a choice of licenses, the files of the one that it is
 *   used under, and that license.
 */
const OVERRIDES = new Map([
  ['harfrust', {
    files: ['LICENSE'],
    source: 'the LICENSE of its repository, https://github.com/harfbuzz/harfrust, at the commit that 0.5.2 was published from (efdae3142ab76a2f1524d72cff9e3dfdc5afd7ca)',
  }],
  ['self_cell', { only: ['LICENSE-APACHE'], under: 'Apache-2.0' }],
]);

/**
 * OpenColorIO, which the crate columbina-ocio links statically into the engine, and the libraries
 * linked with it: those of engine/crates/ocio/build.rs (`library` is its name there, `prefixes` the
 * beginnings of the names of its files there). They are built by engine/crates/ocio/ci/build-ocio.ps1
 * from the release of OpenColorIO, whose CMake fetches the others at the versions that it
 * recommends (share/cmake/modules/FindExtPackages.cmake, OCIO_INSTALL_EXT_PACKAGES=ALL), at the tags
 * and commits below. Their license files, in licenses/ocio/<name>/, are copied unchanged from the
 * sources of that build, besides two notices taken from the head of a source file that is compiled
 * in (FileFormatCDL.cpp of OpenColorIO, the dragonbox of yaml-cpp), with a line saying where from.
 * A new version of OpenColorIO is a new list: its FindExtPackages.cmake, and the license files of each.
 */
export const OCIO_VERSION = '2.6.0';
export const OCIO_LICENSES = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'licenses', 'ocio');
export const OCIO_PARTS = [
  {
    name: 'OpenColorIO', library: 'OpenColorIO', prefixes: ['OpenColorIO'], version: OCIO_VERSION, license: 'BSD-3-Clause',
    source: 'https://github.com/AcademySoftwareFoundation/OpenColorIO, tag v2.6.0',
    files: ['LICENSE', 'THIRD-PARTY.md', 'FileFormatCDL-NOTICE.txt'],
  },
  {
    name: 'yaml-cpp', library: 'yaml-cpp', prefixes: ['yaml-cpp'], version: '0.9.0', license: 'MIT',
    source: 'https://github.com/jbeder/yaml-cpp, tag yaml-cpp-0.9.0 (56e3bb550c91fd7005566f19c079cb7a503223cf)',
    files: ['LICENSE', 'dragonbox-NOTICE.txt'],
  },
  {
    name: 'expat', library: 'expat', prefixes: ['libexpatMT', 'libexpat', 'expat'], version: '2.8.4', license: 'MIT',
    source: 'https://github.com/libexpat/libexpat, tag R_2_8_4 (12cf0b1f25f026a022fe728ad8f7e3d017285b80)',
    files: ['COPYING'],
  },
  {
    name: 'Imath', library: 'Imath', prefixes: ['Imath'], version: '3.2.3', license: 'BSD-3-Clause',
    source: 'https://github.com/AcademySoftwareFoundation/Imath, tag v3.2.3 (5f27ba266d3ea1565e912570c30b5eafc89959f1)',
    files: ['LICENSE.md'],
  },
  {
    name: 'pystring', library: 'pystring', prefixes: ['pystring'], version: '1.1.4', license: 'BSD-3-Clause',
    source: 'https://github.com/imageworks/pystring, tag v1.1.4 (7d16bc814ccb4cad03c300dcb77440034caa84f7)',
    files: ['LICENSE'],
  },
  {
    name: 'minizip-ng', library: 'minizip-ng', prefixes: ['minizip-ng', 'minizip'], version: '4.2.2', license: 'Zlib',
    source: 'https://github.com/zlib-ng/minizip-ng, tag 4.2.2 (7b2387161c542fa9f427352dcdef76097d0d692b)',
    files: ['LICENSE'],
  },
  {
    name: 'zlib', library: 'zlib', prefixes: ['zs', 'zlibstatic', 'zlib', 'z'], version: '1.3.2', license: 'Zlib',
    source: 'https://github.com/madler/zlib, tag v1.3.2 (da607da739fa6047df13e66a2af6b8bec7c2a498)',
    files: ['LICENSE'],
  },
];

/**
 * The OpenColorIO that the engine is linked with: the folder named by --ocio= or OCIO_DIR (as the
 * engine was built), of the version that the notices are of, with a static library of each part.
 * Answers { folder, version }, or ends the packaging, saying why.
 */
export function locateOcio({ args = process.argv.slice(2), env = process.env } = {}) {
  const given = args.find((one) => one.startsWith('--ocio='))?.slice('--ocio='.length) ?? (env.OCIO_DIR || undefined);
  if (!given) {
    throw new Error(`There is no OpenColorIO named, whose notices go with the engine: name the folder that the engine was built with (its OCIO_DIR) by OCIO_DIR or --ocio=. It is made by engine/crates/ocio/ci/build-ocio.ps1, OpenColorIO ${OCIO_VERSION}.`);
  }
  const folder = path.resolve(given);
  const versionFile = path.join(folder, 'lib', 'cmake', 'OpenColorIO', 'OpenColorIOConfigVersion.cmake');
  if (!existsSync(versionFile)) throw new Error(`${folder} has no lib/cmake/OpenColorIO/OpenColorIOConfigVersion.cmake: it is to be the install of OpenColorIO that the engine was built with (OCIO_DIR).`);
  const version = /set\(PACKAGE_VERSION "([^"]+)"\)/.exec(readFileSync(versionFile, 'utf8'))?.[1];
  if (version !== OCIO_VERSION) {
    throw new Error(`The OpenColorIO of ${folder} is ${version ?? 'of no version that it says'}, and the notices of the engine are of OpenColorIO ${OCIO_VERSION} and the libraries that it is built with (OCIO_PARTS of scripts/engine-notices.mjs).`);
  }
  const files = readdirSync(path.join(folder, 'lib'));
  for (const part of OCIO_PARTS) {
    const found = files.some((file) => {
      const stem = /\.lib$/i.test(file) ? file.slice(0, -4) : /^lib.*\.a$/.test(file) ? file.slice(3, -2) : null;
      return stem !== null && part.prefixes.some((prefix) => stem.startsWith(prefix) && /^($|[-_.])/.test(stem.slice(prefix.length)));
    });
    if (!found) throw new Error(`${path.join(folder, 'lib')} has no static library of ${part.name}, which the engine is linked with and whose notices go with it.`);
  }
  return { folder, version };
}

/** Copy the license files of OpenColorIO and its libraries into <out>/licenses/<name>/, and answer the Markdown that lists them. */
export async function writeOcioNotices({ out, from = OCIO_LICENSES }) {
  const rows = [];
  for (const part of OCIO_PARTS) {
    const folder = path.join(out, 'licenses', part.name);
    await rm(folder, { recursive: true, force: true });
    await mkdir(folder, { recursive: true });
    for (const file of part.files) await copyFile(path.join(from, part.name, file), path.join(folder, file));
    rows.push(`| ${part.name} | ${part.version} | ${part.license} | ${part.source} | ${part.files.map((file) => `\`licenses/${part.name}/${file}\``).join(', ')} |`);
  }
  return [
    `The engine is linked statically with OpenColorIO ${OCIO_VERSION} and with the libraries that it is built with,`,
    'each at the version of its source below. Their license files are copied unchanged into `licenses/<name>/`.',
    '',
    '| Library | Version | License | Source | License files |',
    '| --- | --- | --- | --- | --- |',
    ...rows,
    '',
  ].join('\n');
}

/**
 * The C++ libraries that the engine is compiled from, their headers vendored into a crate of it
 * (`engine/crates/stretch/vendor/<name>/`, columbina-stretch, which links them statically): the
 * name of each folder there, its version and source (as `engine/crates/stretch/src/lib.rs` lists
 * them, with the SHA-256 of each archive), and its license files, which are shipped unchanged from
 * that folder. tests/engine-vendored-notices.test.ts holds this list to the folders and to that table.
 */
export const VENDORED_LICENSES = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..', 'engine', 'crates', 'stretch', 'vendor');
export const VENDORED_PARTS = [
  {
    name: 'signalsmith-stretch', title: 'Signalsmith Stretch', version: '1.4.0', license: 'MIT',
    source: 'https://github.com/Signalsmith-Audio/signalsmith-stretch, tag 1.4.0 (a670068d9aeb64913331d5cc29337b19a457a7df)',
    files: ['LICENSE.txt'],
  },
  {
    name: 'signalsmith-linear', title: 'Signalsmith Linear', version: '0.6.4', license: 'MIT',
    source: 'https://github.com/Signalsmith-Audio/linear, tag 0.6.4 (de55e6a50ffcf6f8f43f649692d94691c7025151)',
    files: ['LICENSE.txt'],
  },
];

/** Copy the license files of the vendored libraries into <out>/licenses/<name>/, and answer the Markdown that lists them. */
export async function writeVendoredNotices({ out, from = VENDORED_LICENSES }) {
  const rows = [];
  for (const part of VENDORED_PARTS) {
    const folder = path.join(out, 'licenses', part.name);
    await rm(folder, { recursive: true, force: true });
    await mkdir(folder, { recursive: true });
    for (const file of part.files) await copyFile(path.join(from, part.name, file), path.join(folder, file));
    rows.push(`| ${part.title} | ${part.version} | ${part.license} | ${part.source} | ${part.files.map((file) => `\`licenses/${part.name}/${file}\``).join(', ')} |`);
  }
  return [
    'The engine is compiled with these libraries, from their sources at the versions below. Their license files are',
    'copied unchanged into `licenses/<name>/`.',
    '',
    '| Library | Version | License | Source | License files |',
    '| --- | --- | --- | --- | --- |',
    ...rows,
    '',
  ].join('\n');
}

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
    let license = one.license ?? 'not stated';
    const override = OVERRIDES.get(one.name);
    if (override?.files) {
      if (one.files.length > 0) throw new Error(`${one.name} ${one.version} now brings license files of its own (${one.files.join(', ')}): take it out of the overrides of scripts/engine-notices.mjs.`);
      for (const file of override.files) {
        await copyFile(path.join(CRATE_LICENSES, one.name, file), path.join(folder, file));
      }
      await writeFile(path.join(folder, 'SOURCE.txt'), `${one.name} ${one.version} is published without a license file. Shipped here: ${override.source}.\n`);
      shipped = [...override.files, 'SOURCE.txt'];
    } else if (override?.only) {
      const missing = override.only.filter((file) => !one.files.includes(file));
      if (missing.length > 0) throw new Error(`${one.name} ${one.version} brings no ${missing.join(', ')}: see the overrides of scripts/engine-notices.mjs.`);
      for (const file of override.only) await copyFile(path.join(one.folder, file), path.join(folder, file));
      shipped = override.only;
      license = `${one.license}, used here under ${override.under}`;
    } else if (one.files.length > 0) {
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
    rows.push(`| ${one.name} | ${one.version} | ${license} | ${source} | ${shipped.map((file) => `\`licenses/crates/${named}/${file}\``).join(', ')} |`);
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
