// The pets of the application: packages of a pet, as docs/pets.md in the repository describes them,
// and the file in which the application says what its task is doing, for the pets of other programs.
//
// A package is a folder with `pet.json` and a sheet of frames. The sheet is laid out as the pets of
// Codex are: cells of 192 by 208 pixels, eight to a row, and at least nine rows, one for each way
// of being. Anything else in the folder is left alone. A package that does not hold together is
// not offered, and nothing of it reaches the window of the pet but its picture, as an address of
// data, and the numbers of its rows.
const { mkdirSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } = require('node:fs');
const path = require('node:path');

const CELL_WIDTH = 192;
const CELL_HEIGHT = 208;
const COLUMNS = 8;
/** The rows of a sheet, in their order, each with the way of being that it shows. */
const ROWS = ['idle', 'runRight', 'runLeft', 'wave', 'jump', 'failed', 'waiting', 'thinking', 'review'];
/** How large `pet.json` and the sheet may be, and how many packages are looked at in a folder. */
const MANIFEST_MOST = 64 * 1024;
const SHEET_MOST = 16 * 1024 * 1024;
const PETS_MOST = 100;
/** What the file of the state says a task is doing. */
const STATES = ['idle', 'thinking', 'review', 'waiting', 'failed', 'done'];
const LABEL_MOST = 80;

/** The width and height of a picture in PNG or WebP, from its first bytes, or null for anything else. */
function sizeOf(bytes) {
  if (bytes.length >= 24 && bytes.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))
    && bytes.toString('ascii', 12, 16) === 'IHDR') {
    return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20), type: 'image/png' };
  }
  if (bytes.length >= 30 && bytes.toString('ascii', 0, 4) === 'RIFF' && bytes.toString('ascii', 8, 12) === 'WEBP') {
    const kind = bytes.toString('ascii', 12, 16);
    if (kind === 'VP8X') return { width: 1 + bytes.readUIntLE(24, 3), height: 1 + bytes.readUIntLE(27, 3), type: 'image/webp' };
    if (kind === 'VP8L' && bytes[20] === 0x2f) {
      const bits = bytes.readUInt32LE(21);
      return { width: 1 + (bits & 0x3fff), height: 1 + ((bits >> 14) & 0x3fff), type: 'image/webp' };
    }
    if (kind === 'VP8 ') return { width: bytes.readUInt16LE(26) & 0x3fff, height: bytes.readUInt16LE(28) & 0x3fff, type: 'image/webp' };
  }
  return null;
}

/** A text that a package gives, as far as it may be shown: on one line, and not too long. */
function textOf(value, most) {
  if (typeof value !== 'string') return null;
  const line = value.replace(/[\u0000-\u001f\u007f]+/g, ' ').trim();
  return line ? Array.from(line).slice(0, most).join('') : null;
}

/**
 * The rows that a package names, with their frames and speed, or null where it names none: the
 * window of the pet then counts the frames of each row in the picture.
 *
 * Throws:
 *   Error: What the package names cannot be.
 */
function animationsOf(given, rows) {
  if (given === undefined || given === null) return null;
  if (typeof given !== 'object' || Array.isArray(given)) throw new Error('animations is no object');
  const animations = {};
  for (const name of ROWS) {
    const one = given[name];
    if (one === undefined) continue;
    const { row, frames, fps } = one ?? {};
    if (!Number.isInteger(row) || row < 0 || row >= rows) throw new Error(`the row of ${name} is not in the sheet`);
    if (!Number.isInteger(frames) || frames < 1 || frames > COLUMNS) throw new Error(`${name} has no number of frames from 1 to ${COLUMNS}`);
    if (typeof fps !== 'number' || !(fps >= 1 && fps <= 30)) throw new Error(`${name} has no speed from 1 to 30 frames a second`);
    animations[name] = { row, frames, fps };
  }
  return animations;
}

/**
 * The package of a pet in a folder.
 *
 * Returns:
 *   What is offered of it, with the file of its sheet, which is not handed out.
 *
 * Throws:
 *   Error: The folder holds no package, or one that does not hold together.
 */
function readPet(folder, origin) {
  const manifestFile = path.join(folder, 'pet.json');
  if (statSync(manifestFile).size > MANIFEST_MOST) throw new Error('pet.json is too large');
  const manifest = JSON.parse(readFileSync(manifestFile, 'utf8'));
  if (manifest === null || typeof manifest !== 'object' || Array.isArray(manifest)) throw new Error('pet.json holds no object');
  const id = textOf(manifest.id, 64) ?? path.basename(folder);
  const relative = typeof manifest.spritesheetPath === 'string' ? manifest.spritesheetPath : '';
  const sheet = path.resolve(folder, relative);
  const within = path.relative(folder, sheet);
  if (!relative || within.startsWith('..') || path.isAbsolute(within)) throw new Error('spritesheetPath does not name a file in the folder');
  if (statSync(sheet).size > SHEET_MOST) throw new Error('the sheet is too large');
  const size = sizeOf(readFileSync(sheet).subarray(0, 64));
  if (size === null) throw new Error('the sheet is no PNG or WebP');
  if (size.width !== COLUMNS * CELL_WIDTH || size.height < ROWS.length * CELL_HEIGHT || size.height % CELL_HEIGHT !== 0) {
    throw new Error(`the sheet is ${size.width} by ${size.height}, and not ${COLUMNS} cells of ${CELL_WIDTH} by ${CELL_HEIGHT} to a row, in ${ROWS.length} rows or more`);
  }
  return {
    key: `${origin}/${path.basename(folder)}`,
    id,
    name: textOf(manifest.displayName, 64) ?? id,
    description: textOf(manifest.description, 200) ?? '',
    origin,
    sheet,
    animations: animationsOf(manifest.animations, size.height / CELL_HEIGHT),
  };
}

/** The packages in folders, those that hold together, in the order of the folders and then of their names. */
function listPets(places) {
  const found = [];
  for (const { folder, origin } of places) {
    let names = [];
    try {
      // In the order of their names, the same on every system. A file is no package, and is left out below.
      names = readdirSync(folder).sort();
    } catch { continue; /* A folder that is not there holds no pet. */ }
    for (const name of names.slice(0, PETS_MOST)) {
      try { found.push(readPet(path.join(folder, name), origin)); } catch { /* It is not offered. */ }
    }
  }
  return found;
}

/** The picture of a pet as an address of data, which is all of it that its window is given. */
function sheetOf(pet) {
  const bytes = readFileSync(pet.sheet);
  if (bytes.length > SHEET_MOST) throw new Error('the sheet is too large');
  const size = sizeOf(bytes);
  if (size === null) throw new Error('the sheet is no PNG or WebP');
  return `data:${size.type};base64,${bytes.toString('base64')}`;
}

/**
 * What the main window says the task does, as far as it may be passed on: a state that there is,
 * and a short label and a short bubble on one line. Null for anything else.
 */
function shownOf(given) {
  if (given === null || typeof given !== 'object' || !STATES.includes(given.state)) return null;
  return { state: given.state, label: textOf(given.label, LABEL_MOST) ?? '', bubble: textOf(given.bubble, 2 * LABEL_MOST) ?? '' };
}

/**
 * Write what the task does to the file that other programs read. It is written beside its place
 * and moved there, so that a reader never finds half of it. What cannot be written is let go.
 */
function writeState(file, shown, pid, at) {
  const document = { version: 1, state: shown.state, label: shown.label, at, pid };
  try {
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(`${file}.tmp`, `${JSON.stringify(document)}\n`, 'utf8');
    renameSync(`${file}.tmp`, file);
    return true;
  } catch {
    return false;
  }
}

/** Take the file away, so that its readers know that nobody tells them any more. */
function forgetState(file) {
  try { rmSync(file, { force: true }); } catch { /* It goes at the next start. */ }
}

module.exports = {
  CELL_WIDTH, CELL_HEIGHT, COLUMNS, ROWS, STATES, MANIFEST_MOST, SHEET_MOST,
  sizeOf, animationsOf, readPet, listPets, sheetOf, shownOf, writeState, forgetState,
};
