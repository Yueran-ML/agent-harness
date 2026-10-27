// The engine of the studio in the package: `resources/engine` gets the engine, the libraries of
// FFmpeg (an LGPL shared build) that it and the ffmpeg of its proxies load, that ffmpeg, and the
// notices and licenses of all of them. electron/package.mjs runs it once the application is packaged.
//
// Where they are taken from, each named by an argument of package.mjs or else by a variable:
//   --engine-build=<dir>  COLUMBINA_ENGINE_BUILD  the build of engine/, by default engine/target/release
//   --ffmpeg=<dir>        COLUMBINA_FFMPEG_LGPL   the LGPL shared build of FFmpeg 8.1 (with bin/ and
//                                                 LICENSE.txt, as BtbN's), by default FFMPEG_DIR
// What is missing ends the packaging, saying what.
//
// Which libraries: those that the engine and ffmpeg.exe import (the import tables of their PE
// files, read here, delay-loaded ones too), and what those import in turn, as long as it is a
// file of the FFmpeg build. Everything else they import must be a library that every Windows 10
// and 11 has (WINDOWS_LIBRARIES, and the api-ms-win-* sets). The engine is linked with the C
// runtime statically (engine/.cargo/config.toml): an engine that imports a VC runtime is refused.
//
// Which license: every library of FFmpeg says its own in a string that configure builds into it,
// `libavcodec license: LGPL version 3 or later`; a library without one, or with any other license
// (`GPL version …`, `nonfree and unredistributable`) is refused, and so is an ffmpeg whose
// `-buildconf` has --enable-gpl or --enable-nonfree, or whose `-L` is not the LGPL's.
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
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { hostTarget, rustCrates, writeCrateNotices } from '../scripts/engine-notices.mjs';

/** The files of the engine's folder on each system, as far as they are known. */
export const LAYOUT = {
  win32: { engine: 'columbina-engine.exe', ffmpeg: 'ffmpeg.exe', libraries: 'bin' },
  darwin: { engine: 'columbina-engine', ffmpeg: 'ffmpeg', libraries: 'lib', todo: 'macOS' },
  linux: { engine: 'columbina-engine', ffmpeg: 'ffmpeg', libraries: 'lib', todo: 'Linux' },
};

/** The C runtime of Microsoft: not a part of Windows. The engine is not to need it. */
const VC_RUNTIME = /^(vcruntime|msvcp|concrt|vccorlib|vcomp)\d|^ucrtbased\.dll$/;

/**
 * The libraries of Windows that the engine and ffmpeg may import: each is in every Windows 10 and
 * 11 on x64. A new one is added here once it is known to be so, not because the machine that
 * packages has it in System32 (where a VC runtime or a driver may have put anything).
 */
const WINDOWS_LIBRARIES = new Set([
  'advapi32.dll', 'avicap32.dll', 'avrt.dll', 'bcrypt.dll', 'bcryptprimitives.dll', 'cfgmgr32.dll', 'combase.dll',
  'crypt32.dll', 'd2d1.dll', 'd3d11.dll', 'd3d12.dll', 'd3dcompiler_47.dll', 'dnsapi.dll', 'dwmapi.dll', 'dwrite.dll',
  'dxgi.dll', 'gdi32.dll', 'imm32.dll', 'iphlpapi.dll', 'kernel32.dll', 'kernelbase.dll', 'mf.dll', 'mfplat.dll',
  'mfreadwrite.dll', 'mmdevapi.dll', 'msimg32.dll', 'ncrypt.dll', 'ntdll.dll', 'ole32.dll', 'oleaut32.dll',
  'propsys.dll', 'psapi.dll', 'rpcrt4.dll', 'secur32.dll', 'setupapi.dll', 'shcore.dll', 'shell32.dll',
  'shlwapi.dll', 'user32.dll', 'userenv.dll', 'usp10.dll', 'uxtheme.dll', 'version.dll', 'winmm.dll', 'ws2_32.dll',
]);

/** The libraries of FFmpeg, each of which says its license: libavcodec of avcodec-62.dll. */
const FFMPEG_LIBRARY = /^(avcodec|avformat|avutil|avfilter|avdevice|swscale|swresample|postproc)-\d+\.dll$/i;
const LGPL = new Set(['LGPL version 2.1 or later', 'LGPL version 3 or later']);

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

/**
 * The license that a library of FFmpeg says it is under, which must be the LGPL: the string
 * `lib<name> license: …` that configure builds into it. Answers it, or ends the packaging.
 */
export function libraryLicense(file) {
  const library = `lib${FFMPEG_LIBRARY.exec(path.basename(file))?.[1]?.toLowerCase() ?? '?'}`;
  const said = [...readFileSync(file).toString('latin1').matchAll(/\b(lib\w+) license: ([\x20-\x7e]{1,80})/g)]
    .map((match) => ({ library: match[1], license: match[2] }));
  const others = said.filter((one) => !LGPL.has(one.license));
  if (others.length > 0) {
    throw new Error(`${file} is not of the LGPL, and is not to be shipped: it says ${others.map((one) => `"${one.library} license: ${one.license}"`).join(', ')}.`);
  }
  const own = said.find((one) => one.library === library);
  if (!own) throw new Error(`${file} does not say its license ("${library} license: …"), and is not shipped without it.`);
  return own.license;
}

/**
 * The libraries to bring beside the engine and ffmpeg.exe: name (as the folder has it) to the file
 * it is copied from. What is not found, or not allowed, ends the packaging.
 */
async function windowsLibraries(programs, ffmpegBin) {
  const inBuild = new Map((await readdir(ffmpegBin)).filter((name) => /\.dll$/i.test(name)).map((name) => [name.toLowerCase(), path.join(ffmpegBin, name)]));
  const brought = new Map();
  const ofWindows = new Set();
  const waiting = [...programs];
  while (waiting.length > 0) {
    const file = waiting.pop();
    for (const imported of peImports(await readFile(file), file)) {
      const name = imported.toLowerCase();
      if (brought.has(name) || ofWindows.has(name)) continue;
      if (VC_RUNTIME.test(name)) {
        throw new Error(`${file} imports ${imported}, a VC runtime that Windows does not bring. The engine is to be linked with the C runtime statically (engine/.cargo/config.toml: target-feature=+crt-static); a RUSTFLAGS in the environment takes the place of that file.`);
      }
      if (inBuild.has(name)) {
        if (!FFMPEG_LIBRARY.test(name)) throw new Error(`${path.basename(file)} imports ${imported} of the FFmpeg build, which is not one of the libraries of FFmpeg whose license is checked.`);
        libraryLicense(inBuild.get(name));
        brought.set(name, inBuild.get(name));
        waiting.push(inBuild.get(name));
      } else if (name.startsWith('api-ms-win-') || WINDOWS_LIBRARIES.has(name)) {
        ofWindows.add(name);
      } else {
        throw new Error(`${path.basename(file)} imports ${imported}, which is neither in ${ffmpegBin} nor a library that every Windows has (WINDOWS_LIBRARIES of electron/engine-bundle.mjs).`);
      }
    }
  }
  return { brought, ofWindows };
}

/** The libraries of the C runtime that a program imports, which the engine of the package may not. */
export function runtimeImports(file) {
  return peImports(readFileSync(file), file).filter((name) => VC_RUNTIME.test(name.toLowerCase()) || /^api-ms-win-crt-/i.test(name));
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
  // What the program says of its own license (fftools of a GPL build say the GPL's).
  const license = ffmpegSays(ffmpeg, '-L');
  if (!/GNU Lesser General Public License/.test(license) || /nonfree|unredistributable/i.test(license)) {
    throw new Error(`${ffmpeg} -L does not say that it is under the GNU Lesser General Public License, and it is not to be shipped:\n${license}`);
  }
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
  // Not even copied if it is not an LGPL build: the program, and each of its libraries.
  lgplConfiguration(ffmpeg);
  const libraries = readdirSync(ffmpegBin).filter((name) => FFMPEG_LIBRARY.test(name));
  if (libraries.length === 0) throw new Error(`${ffmpegBin} has no libraries of FFmpeg: it is to be a shared build.`);
  for (const name of libraries) libraryLicense(path.join(ffmpegBin, name));
  // Nor is an engine that needs a VC runtime, or the C runtime of Windows, which it is to have in itself.
  const runtime = runtimeImports(engine);
  if (runtime.length > 0) {
    throw new Error(`${engine} imports ${runtime.join(', ')}: it is to be linked with the C runtime statically (engine/.cargo/config.toml: target-feature=+crt-static); a RUSTFLAGS in the environment takes the place of that file.`);
  }
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
  // What is written for those who keep the repository (in <!-- -->) is not shipped.
  const notice = (await readFile(path.join(desktop, 'THIRD_PARTY_ENGINE.md'), 'utf8')).replace(/<!--[\s\S]*?-->\s*/g, '');
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
