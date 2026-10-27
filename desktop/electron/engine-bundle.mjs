// The engine of the studio in the package: `resources/engine` gets the engine, the libraries of
// FFmpeg (an LGPL shared build) that it and the ffmpeg of its proxies load, that ffmpeg, the C
// runtime of Microsoft that the engine is linked with, and the notices and licenses of all of them.
// electron/package.mjs runs it once the application is packaged.
//
// Where they are taken from, each named by an argument of package.mjs or else by a variable:
//   --engine-build=<dir>  COLUMBINA_ENGINE_BUILD  the build of engine/, by default engine/target/release
//   --ffmpeg=<dir>        COLUMBINA_FFMPEG_LGPL   the LGPL shared build of FFmpeg 8.1 (with bin/ and
//                                                 LICENSE.txt, as BtbN's), by default FFMPEG_DIR
//   COLUMBINA_VC_REDIST   the folder with vcruntime140.dll, by default the VC\Redist of the newest
//                         Visual Studio that vswhere finds
// What is missing ends the packaging, saying what.
//
// Which libraries: those that the engine and ffmpeg.exe import (the import tables of their PE
// files, read here, delay-loaded ones too), and what those import in turn, as long as it is a
// file of the FFmpeg build or the C runtime of Microsoft. Everything else they import must be a
// library of Windows: an api-ms-win-* set, or a file of System32 that is not the C runtime.
//
// macOS and Linux. The engine of macOS is built in CI against ffmpeg@8 of Homebrew, which is
// configured with --enable-gpl, and is not to be shipped: the engine is left out of the package of
// macOS (the view of the studio is not supported there yet, and the application says it has no
// engine). To bring it, the engine has to be built against an LGPL build of FFmpeg 8.1 for macOS
// (one configured without --enable-gpl and --enable-nonfree; BtbN makes none for macOS), and
// resources/engine would hold `columbina-engine`, `ffmpeg`, and libavcodec.62.dylib,
// libavformat.62.dylib, libavutil.60.dylib, libswscale.9.dylib, libswresample.6.dylib (plus
// libavfilter.11 and libavdevice.62 for ffmpeg), each with an install name of
// @loader_path/<name>, found with `otool -L`, and signed with the application. On Linux (no package
// is made yet) the same names end in .so.<major>, found with `readelf -d`, with an RPATH of $ORIGIN.
import { spawnSync } from 'node:child_process';
import { copyFile, mkdir, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { hostTarget, rustCrates, writeCrateNotices } from '../scripts/engine-notices.mjs';

/** The files of the engine's folder on each system, as far as they are known. */
export const LAYOUT = {
  win32: { engine: 'columbina-engine.exe', ffmpeg: 'ffmpeg.exe', libraries: 'bin' },
  darwin: { engine: 'columbina-engine', ffmpeg: 'ffmpeg', libraries: 'lib', todo: 'macOS' },
  linux: { engine: 'columbina-engine', ffmpeg: 'ffmpeg', libraries: 'lib', todo: 'Linux' },
};

/** The C runtime of Microsoft: not a part of Windows, and redistributable beside a program. */
const VC_RUNTIME = new Set([
  'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'msvcp140_1.dll', 'msvcp140_2.dll',
  'msvcp140_atomic_wait.dll', 'msvcp140_codecvt_ids.dll', 'concrt140.dll', 'vccorlib140.dll',
]);

/** The value of --name=value among the arguments, or of the variable. */
function named(args, flag, variable) {
  const given = args.find((one) => one.startsWith(`${flag}=`));
  return given ? given.slice(flag.length + 1) : process.env[variable] || undefined;
}

/** The names of the libraries that a PE file imports, delay-loaded ones too. */
export function peImports(buffer, file = 'the file') {
  const bad = (why) => new Error(`${file} is not a PE file that can be read: ${why}`);
  if (buffer.length < 0x40 || buffer.readUInt16LE(0) !== 0x5a4d) throw bad('no MZ');
  const pe = buffer.readUInt32LE(0x3c);
  if (buffer.readUInt32LE(pe) !== 0x4550) throw bad('no PE signature');
  const coff = pe + 4;
  const sections = buffer.readUInt16LE(coff + 2);
  const optionalSize = buffer.readUInt16LE(coff + 16);
  const optional = coff + 20;
  const wide = buffer.readUInt16LE(optional) === 0x20b;
  const imageBase = wide ? Number(buffer.readBigUInt64LE(optional + 24)) : buffer.readUInt32LE(optional + 28);
  const directories = buffer.readUInt32LE(optional + (wide ? 108 : 92));
  const directory = (index) => {
    if (index >= directories) return { rva: 0, size: 0 };
    const at = optional + (wide ? 112 : 96) + index * 8;
    return { rva: buffer.readUInt32LE(at), size: buffer.readUInt32LE(at + 4) };
  };
  const table = optional + optionalSize;
  const offset = (rva) => {
    for (let index = 0; index < sections; index += 1) {
      const at = table + index * 40;
      const size = Math.max(buffer.readUInt32LE(at + 8), buffer.readUInt32LE(at + 16));
      const address = buffer.readUInt32LE(at + 12);
      if (rva >= address && rva < address + size) return rva - address + buffer.readUInt32LE(at + 20);
    }
    throw bad(`the address ${rva.toString(16)} is in no section`);
  };
  const text = (rva) => {
    const at = offset(rva);
    return buffer.toString('latin1', at, buffer.indexOf(0, at));
  };
  const names = [];
  const imports = directory(1);
  if (imports.rva) {
    for (let at = offset(imports.rva); buffer.readUInt32LE(at + 12) !== 0; at += 20) names.push(text(buffer.readUInt32LE(at + 12)));
  }
  const delayed = directory(13);
  if (delayed.rva) {
    for (let at = offset(delayed.rva); buffer.readUInt32LE(at + 4) !== 0; at += 32) {
      // Attributes of 1: the addresses are relative; of 0 (an old linker), they are virtual.
      const name = buffer.readUInt32LE(at + 4);
      names.push(text(buffer.readUInt32LE(at) & 1 ? name : name - imageBase));
    }
  }
  return names;
}

/** The folder of the C runtime of the newest Visual Studio with the tools of C++, or null. */
async function vcRuntimeFolder() {
  if (process.env.COLUMBINA_VC_REDIST) return process.env.COLUMBINA_VC_REDIST;
  const vswhere = path.join(process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)', 'Microsoft Visual Studio', 'Installer', 'vswhere.exe');
  if (!existsSync(vswhere)) return null;
  const found = spawnSync(vswhere, ['-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath'], { encoding: 'utf8' });
  const installation = found.stdout?.trim().split(/\r?\n/)[0];
  if (!installation) return null;
  const redist = path.join(installation, 'VC', 'Redist', 'MSVC');
  const versions = (await readdir(redist).catch(() => []))
    .filter((name) => /^\d+(\.\d+)+$/.test(name))
    .sort((one, other) => other.localeCompare(one, undefined, { numeric: true }));
  for (const version of versions) {
    const x64 = path.join(redist, version, 'x64');
    for (const crt of (await readdir(x64).catch(() => [])).filter((name) => /^Microsoft\.VC\d+\.CRT$/.test(name))) {
      if (existsSync(path.join(x64, crt, 'vcruntime140.dll'))) return path.join(x64, crt);
    }
  }
  return null;
}

/**
 * The libraries to bring beside the engine and ffmpeg.exe: name (as the folder has it) to the file
 * it is copied from. What is not found ends the packaging.
 */
async function windowsLibraries(programs, ffmpegBin) {
  const system32 = path.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32');
  const inBuild = new Map((await readdir(ffmpegBin)).filter((name) => /\.dll$/i.test(name)).map((name) => [name.toLowerCase(), path.join(ffmpegBin, name)]));
  const brought = new Map();
  const ofWindows = new Set();
  let vcFolder;
  const waiting = [...programs];
  while (waiting.length > 0) {
    const file = waiting.pop();
    for (const imported of peImports(await readFile(file), file)) {
      const name = imported.toLowerCase();
      if (brought.has(name) || ofWindows.has(name)) continue;
      if (inBuild.has(name)) {
        brought.set(name, inBuild.get(name));
        waiting.push(inBuild.get(name));
      } else if (VC_RUNTIME.has(name)) {
        if (vcFolder === undefined) vcFolder = await vcRuntimeFolder();
        const from = vcFolder && path.join(vcFolder, name);
        if (!from || !existsSync(from)) {
          throw new Error(`${path.basename(file)} needs ${imported}, the C runtime of Microsoft, and it was not found: name the folder that has it with COLUMBINA_VC_REDIST (VC\\Redist\\MSVC\\<version>\\x64\\Microsoft.VC143.CRT of Visual Studio).`);
        }
        brought.set(name, from);
        waiting.push(from);
      } else if (/^(api|ext)-ms-win-/.test(name) || existsSync(path.join(system32, name))) {
        ofWindows.add(name);
      } else {
        throw new Error(`${path.basename(file)} needs ${imported}, which is neither in ${ffmpegBin} nor a library of Windows.`);
      }
    }
  }
  return { brought, ofWindows };
}

/** The environment of a program of the package as a user without any toolchain has it: Windows, and nothing else, on PATH. */
export function cleanEnvironment() {
  const root = process.env.SystemRoot ?? 'C:\\Windows';
  const env = {};
  for (const name of ['SystemRoot', 'SystemDrive', 'windir', 'TEMP', 'TMP', 'USERPROFILE', 'LOCALAPPDATA', 'APPDATA', 'ComSpec', 'PATHEXT', 'NUMBER_OF_PROCESSORS', 'PROCESSOR_ARCHITECTURE', 'OS']) {
    if (process.env[name] !== undefined) env[name] = process.env[name];
  }
  env.PATH = [path.join(root, 'System32'), root, path.join(root, 'System32', 'Wbem')].join(path.delimiter);
  return env;
}

/** What `ffmpeg -hide_banner <flag>` writes, run from the folder of the package with nothing else on PATH. */
function ffmpegSays(ffmpeg, flag) {
  const result = spawnSync(ffmpeg, ['-hide_banner', flag], { encoding: 'utf8', env: cleanEnvironment(), cwd: path.dirname(ffmpeg), timeout: 60_000 });
  if (result.error) throw new Error(`${ffmpeg} ${flag} could not be run: ${result.error.message}`);
  if (result.status !== 0) throw new Error(`${ffmpeg} ${flag} ended with ${result.status ?? result.signal} (0xc0000135 is a library that is not found):\n${result.stderr}`);
  return `${result.stdout}${result.stderr}`;
}

/** The configuration of an ffmpeg, which must be that of an LGPL build. */
export function lgplConfiguration(ffmpeg) {
  const said = ffmpegSays(ffmpeg, '-buildconf');
  // One option a line, mostly; some lines hold two.
  const options = [...said.matchAll(/(?:^|\s)(--\S+)/g)].map((match) => match[1]);
  if (options.length === 0) throw new Error(`${ffmpeg} -buildconf names no options:\n${said}`);
  const forbidden = options.filter((option) => option === '--enable-gpl' || option === '--enable-nonfree');
  if (forbidden.length > 0) throw new Error(`${ffmpeg} is not an LGPL build: it is configured with ${forbidden.join(' and ')}, and is not to be shipped.`);
  return options;
}

/** The notice of FFmpeg that only the build can tell: its version, where its source is, how it was configured. */
function ffmpegNotice(versionLine, options) {
  // n8.1.3-24-g92611900bb-20261025: the release tag n8.1.3, 24 commits after it, at commit 92611900bb.
  const described = /^ffmpeg version (\S+)/.exec(versionLine)?.[1] ?? 'unknown';
  const revision = /^(n[\d.]+)(?:-(\d+)-g([0-9a-f]+))?/.exec(described);
  const where = revision
    ? revision[3]
      ? `commit \`${revision[3]}\`, ${revision[2]} commits after the tag \`${revision[1]}\` on its release branch: https://github.com/FFmpeg/FFmpeg/commit/${revision[3]} (the same as git://source.ffmpeg.org/ffmpeg.git)`
      : `the tag \`${revision[1]}\`: https://github.com/FFmpeg/FFmpeg/tree/${revision[1]}`
    : 'the revision in the version line above, in https://git.ffmpeg.org/ffmpeg.git';
  const extra = options.find((option) => option.startsWith('--extra-version='))?.slice('--extra-version='.length);
  return [
    '## This build of FFmpeg',
    '',
    `\`ffmpeg -version\` of the files in this folder says: \`${versionLine.trim()}\``,
    '',
    `- The source of FFmpeg that it is built from is ${where}.`,
    `- It is the \`win64-lgpl-shared-8.1\` build of BtbN${extra ? ` made on ${extra.slice(0, 4)}-${extra.slice(4, 6)}-${extra.slice(6, 8)}` : ''}: the scripts that`,
    '  built it, and with them the sources and versions of the libraries that its `--enable-lib…` options',
    '  link into the DLLs, are https://github.com/BtbN/FFmpeg-Builds (`scripts.d/`), as they were on that day.',
    '- It is configured (`ffmpeg -hide_banner -buildconf`) with:',
    '',
    '```',
    ...options,
    '```',
    '',
  ].join('\n');
}

/**
 * Where the engine and the FFmpeg to bring with it are, checked before anything is packaged: both
 * are there, and the FFmpeg is an LGPL build. Answers them, or why no engine is brought.
 */
export function locateEngine({ desktop, args = process.argv.slice(2), platform = process.platform }) {
  const layout = LAYOUT[platform];
  if (!layout) throw new Error(`There is no layout of the engine for ${platform}.`);
  if (layout.todo) {
    const why = platform === 'darwin'
      ? 'the engine of macOS is built against ffmpeg@8 of Homebrew, a GPL build, which is not shipped; it needs an LGPL build of FFmpeg 8.1 for macOS first (see electron/engine-bundle.mjs). The view of the studio is not supported on macOS yet.'
      : `packages of ${layout.todo} bring no engine yet.`;
    return { skipped: why };
  }
  const repository = path.resolve(desktop, '..');
  const build = path.resolve(named(args, '--engine-build', 'COLUMBINA_ENGINE_BUILD') ?? path.join(repository, 'engine', 'target', 'release'));
  const ffmpegRoot = named(args, '--ffmpeg', 'COLUMBINA_FFMPEG_LGPL') ?? process.env.FFMPEG_DIR;
  const engine = path.join(build, layout.engine);
  if (!existsSync(engine)) {
    throw new Error(`There is no engine at ${engine}. Build it (cargo build --release --locked --package columbina-server --bin columbina-engine in engine/), or name its folder with --engine-build= or COLUMBINA_ENGINE_BUILD.`);
  }
  if (!ffmpegRoot) {
    throw new Error('There is no FFmpeg to bring with the engine: name the LGPL shared build of FFmpeg 8.1 that it is built with by --ffmpeg=, COLUMBINA_FFMPEG_LGPL or FFMPEG_DIR.');
  }
  const root = path.resolve(ffmpegRoot);
  const ffmpegBin = path.join(root, layout.libraries);
  const ffmpeg = path.join(ffmpegBin, layout.ffmpeg);
  const ffmpegLicense = path.join(root, 'LICENSE.txt');
  for (const file of [ffmpeg, ffmpegLicense]) {
    if (!existsSync(file)) throw new Error(`The FFmpeg of ${root} has no ${path.relative(root, file)}: it is to be an LGPL shared build of BtbN.`);
  }
  // Not even copied if it is not an LGPL build.
  lgplConfiguration(ffmpeg);
  return { platform, layout, repository, engine, ffmpegBin, ffmpeg, ffmpegLicense };
}

/**
 * Put the engine, its libraries, the ffmpeg of its proxies and their notices into
 * `<resources>/engine`, as locateEngine() found them. Answers what was brought.
 */
export async function bundleEngine({ resources, desktop, located }) {
  if (located.skipped) {
    console.warn(`The package has no engine of the studio: ${located.skipped}`);
    return located;
  }
  const { platform, layout, repository, engine, ffmpegBin, ffmpeg, ffmpegLicense } = located;
  const { brought, ofWindows } = await windowsLibraries([engine, ffmpeg], ffmpegBin);
  const folder = path.join(resources, 'engine');
  await rm(folder, { recursive: true, force: true });
  await mkdir(folder, { recursive: true });
  const files = [[layout.engine, engine], [layout.ffmpeg, ffmpeg], ...[...brought.values()].map((from) => [path.basename(from), from])];
  for (const [name, from] of files) await copyFile(from, path.join(folder, name));

  // The ffmpeg of the package, run with nothing but Windows on PATH: it must start with the
  // libraries beside it, and be of the LGPL.
  const bundledFfmpeg = path.join(folder, layout.ffmpeg);
  const options = lgplConfiguration(bundledFfmpeg);
  const versionLine = ffmpegSays(bundledFfmpeg, '-version').split(/\r?\n/)[0];

  // The notices: the one of the repository, then what only this build can tell.
  const licenses = path.join(folder, 'licenses');
  await mkdir(path.join(licenses, 'FFmpeg'), { recursive: true });
  await copyFile(ffmpegLicense, path.join(licenses, 'FFmpeg', 'LICENSE.txt'));
  await copyFile(path.join(desktop, 'licenses', 'GPL-3.0.txt'), path.join(licenses, 'FFmpeg', 'GPL-3.0.txt'));
  const crates = await rustCrates({ engine: path.join(repository, 'engine'), target: hostTarget(platform) });
  const crateList = await writeCrateNotices({ crates, out: folder });
  const notice = await readFile(path.join(desktop, 'THIRD_PARTY_ENGINE.md'), 'utf8');
  await writeFile(path.join(folder, 'THIRD_PARTY_ENGINE.md'), [
    notice.trimEnd(), '',
    ffmpegNotice(versionLine, options),
    '## The files of this folder', '',
    ...files.map(([name]) => `- \`${name}\``).sort(),
    '', `They load besides, from Windows itself: ${[...ofWindows].sort().join(', ')}.`, '',
    '## Rust crates in the engine', '',
    crateList,
  ].join('\n'));

  const size = (await Promise.all(files.map(([name]) => stat(path.join(folder, name))))).reduce((sum, one) => sum + one.size, 0);
  console.log(`The engine is in ${folder}: ${files.map(([name]) => name).sort().join(', ')} (${Math.round(size / 1024 / 1024)} MB), and the notices of FFmpeg and of ${crates.length} crates.`);
  return { folder, files: files.map(([name]) => name).sort(), libraries: [...brought.keys()].sort(), ofWindows: [...ofWindows].sort(), versionLine, crates: crates.length };
}
