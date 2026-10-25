/** The packages of pets, as the shell reads them, and the file that tells other programs what the task does. */
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import pets from '../electron/pets.cjs';
import { drawPets } from '../scripts/draw-pets.mjs';

const folders: string[] = [];
afterEach(() => { for (const folder of folders.splice(0)) rmSync(folder, { recursive: true, force: true }); });
const scratch = () => {
  const folder = mkdtempSync(path.join(tmpdir(), 'columbina-pets-'));
  folders.push(folder);
  return folder;
};

/** The first bytes of a PNG of a size, which is all that the shell reads of it to know its size. */
function png(width: number, height: number) {
  const head = Buffer.alloc(33);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(head, 0);
  head.writeUInt32BE(13, 8);
  head.write('IHDR', 12, 'ascii');
  head.writeUInt32BE(width, 16);
  head.writeUInt32BE(height, 20);
  return head;
}
/** The first bytes of a WebP of a size, in each of its three kinds. */
function webp(kind: 'VP8X' | 'VP8L' | 'VP8 ', width: number, height: number) {
  const head = Buffer.alloc(40);
  head.write('RIFF', 0, 'ascii');
  head.write('WEBP', 8, 'ascii');
  head.write(kind, 12, 'ascii');
  if (kind === 'VP8X') { head.writeUIntLE(width - 1, 24, 3); head.writeUIntLE(height - 1, 27, 3); }
  if (kind === 'VP8L') { head[20] = 0x2f; head.writeUInt32LE((width - 1) | ((height - 1) << 14), 21); }
  if (kind === 'VP8 ') { head.writeUInt16LE(width, 26); head.writeUInt16LE(height, 28); }
  return head;
}

/** A package in a folder: its manifest, and a sheet of a size. */
function pack(parent: string, name: string, manifest: unknown, sheet: Buffer | null = png(1536, 1872), file = 'sheet.png') {
  const folder = path.join(parent, name);
  mkdirSync(folder, { recursive: true });
  writeFileSync(path.join(folder, 'pet.json'), typeof manifest === 'string' ? manifest : JSON.stringify(manifest));
  if (sheet) writeFileSync(path.join(folder, file), sheet);
  return folder;
}
const manifest = (more: Record<string, unknown> = {}) => ({ id: 'mochi', displayName: 'Mochi', description: 'A round friend.', spritesheetPath: 'sheet.png', ...more });

describe('the size of a picture', () => {
  it('is read from the head of a PNG and of each kind of WebP', () => {
    expect(pets.sizeOf(png(1536, 1872))).toEqual({ width: 1536, height: 1872, type: 'image/png' });
    expect(pets.sizeOf(webp('VP8X', 1536, 2288))).toEqual({ width: 1536, height: 2288, type: 'image/webp' });
    expect(pets.sizeOf(webp('VP8L', 1536, 1872))).toEqual({ width: 1536, height: 1872, type: 'image/webp' });
    expect(pets.sizeOf(webp('VP8 ', 1536, 1872))).toEqual({ width: 1536, height: 1872, type: 'image/webp' });
  });

  it('is nothing for anything else', () => {
    expect(pets.sizeOf(Buffer.from('GIF89a and more bytes than a head has, to be sure'))).toBeNull();
    expect(pets.sizeOf(png(1, 1).subarray(0, 20))).toBeNull();
    const notIhdr = png(1536, 1872);
    notIhdr.write('IDAT', 12, 'ascii');
    expect(pets.sizeOf(notIhdr)).toBeNull();
    const lossless = webp('VP8L', 1536, 1872);
    lossless[20] = 0;
    expect(pets.sizeOf(lossless)).toBeNull();
    const other = webp('VP8X', 1, 1);
    other.write('ALPH', 12, 'ascii');
    expect(pets.sizeOf(other)).toBeNull();
    expect(pets.sizeOf(webp('VP8X', 1, 1).subarray(0, 29))).toBeNull();
  });
});

describe('a package of a pet', () => {
  it('is read with its names, its sheet, and the rows that it names', () => {
    const folder = pack(scratch(), 'mochi', manifest({ animations: { idle: { row: 0, frames: 6, fps: 5 }, unknown: { row: 99 } } }));
    expect(pets.readPet(folder, 'user')).toEqual({
      key: 'user/mochi', id: 'mochi', name: 'Mochi', description: 'A round friend.', origin: 'user',
      sheet: path.join(folder, 'sheet.png'), animations: { idle: { row: 0, frames: 6, fps: 5 } },
    });
  });

  it('is known by its folder, whatever it calls itself', () => {
    const pet = pets.readPet(pack(scratch(), 'folder-name', manifest({ id: 'mochi' })), 'user');
    expect([pet.key, pet.id]).toEqual(['user/folder-name', 'mochi']);
  });

  it('is named by its folder where it names itself in nothing, and its names are put on one line and cut', () => {
    const folder = pack(scratch(), 'plain', { spritesheetPath: 'sheet.png', displayName: ` A\nname ${'x'.repeat(80)}`, description: 7 });
    const pet = pets.readPet(folder, 'builtin');
    expect([pet.key, pet.id, pet.name.length, pet.name.startsWith('A name x'), pet.description, pet.animations])
      .toEqual(['builtin/plain', 'plain', 64, true, '', null]);
    expect(pets.readPet(pack(scratch(), 'blank', { id: '  ', displayName: '', spritesheetPath: 'sheet.png' }), 'user').name).toBe('blank');
  });

  it('takes a sheet with more rows than nine, whose rows its animations may name, and one in WebP', () => {
    const tall = pets.readPet(pack(scratch(), 'tall', manifest({ animations: { idle: { row: 10, frames: 8, fps: 4 } } }), png(1536, 2288)), 'user');
    expect(tall.animations).toEqual({ idle: { row: 10, frames: 8, fps: 4 } });
    const parent = scratch();
    expect(pets.readPet(pack(parent, 'webp', manifest({ spritesheetPath: 'sheet.webp' }), webp('VP8X', 1536, 1872), 'sheet.webp'), 'user').sheet)
      .toBe(path.join(parent, 'webp', 'sheet.webp'));
  });

  it.each([
    ['no object', '[1, 2]', 'pet.json holds no object'],
    ['a sheet outside its folder', manifest({ spritesheetPath: '../sheet.png' }), 'does not name a file in the folder'],
    ['no sheet named', manifest({ spritesheetPath: undefined }), 'does not name a file in the folder'],
    ['a sheet at a path of its own', manifest({ spritesheetPath: path.resolve('/elsewhere/sheet.png') }), 'does not name a file in the folder'],
    ['a sheet that is no picture', manifest({ spritesheetPath: 'pet.json' }), 'no PNG or WebP'],
    ['animations that are no object', manifest({ animations: [1] }), 'animations is no object'],
    ['a row outside the sheet', manifest({ animations: { idle: { row: 9, frames: 1, fps: 5 } } }), 'is not in the sheet'],
    ['a row that is no number', manifest({ animations: { idle: { row: 'one', frames: 1, fps: 5 } } }), 'is not in the sheet'],
    ['too many frames', manifest({ animations: { wave: { row: 3, frames: 9, fps: 5 } } }), 'number of frames'],
    ['no frames', manifest({ animations: { wave: { row: 3, frames: 0, fps: 5 } } }), 'number of frames'],
    ['too fast', manifest({ animations: { jump: { row: 4, frames: 2, fps: 31 } } }), 'speed'],
    ['no speed', manifest({ animations: { jump: { row: 4, frames: 2 } } }), 'speed'],
    ['an animation that is nothing', manifest({ animations: { jump: null } }), 'is not in the sheet'],
  ])('is refused for %s', (_what, given, why) => {
    expect(() => pets.readPet(pack(scratch(), 'bad', given), 'user')).toThrow(why);
  });

  it.each([
    ['a sheet of another width', png(1535, 1872)],
    ['a sheet of too few rows', png(1536, 1664)],
    ['a sheet whose rows are not whole', png(1536, 1880)],
    ['a sheet that is no picture inside', Buffer.from('not a picture at all, only words')],
  ])('is refused for %s', (_what, sheet) => {
    expect(() => pets.readPet(pack(scratch(), 'bad', manifest(), sheet), 'user')).toThrow();
  });

  it('is refused where a file is too large to be read', () => {
    const parent = scratch();
    const big = pack(parent, 'big', manifest());
    writeFileSync(path.join(big, 'pet.json'), ' '.repeat(pets.MANIFEST_MOST + 1));
    expect(() => pets.readPet(big, 'user')).toThrow('pet.json is too large');
    const huge = pack(parent, 'huge', manifest(), Buffer.concat([png(1536, 1872), Buffer.alloc(pets.SHEET_MOST)]));
    expect(() => pets.readPet(huge, 'user')).toThrow('the sheet is too large');
    expect(() => pets.readPet(path.join(parent, 'missing'), 'user')).toThrow();
  });
});

describe('the pets that can be chosen', () => {
  it('are the packages that hold together, folder by folder, each folder in the order of its names', () => {
    const builtIn = scratch();
    const mine = scratch();
    // In the order of the code of their names, as on every system: a capital before a small letter.
    pack(builtIn, 'b', manifest({ id: 'b' }));
    pack(builtIn, 'a', manifest({ id: 'a' }));
    pack(builtIn, 'C', manifest({ id: 'C' }));
    pack(mine, 'broken', '{');
    writeFileSync(path.join(mine, 'a file'), 'no folder');
    pack(mine, 'mine', manifest({ id: 'mine' }));
    const found = pets.listPets([
      { folder: builtIn, origin: 'builtin' }, { folder: path.join(mine, 'not there'), origin: 'user' }, { folder: mine, origin: 'user' },
    ]);
    expect(found.map((pet: { key: string }) => pet.key)).toEqual(['builtin/C', 'builtin/a', 'builtin/b', 'user/mine']);
  });

  it('are those that come with the application, as the script draws them', () => {
    const drawn = scratch();
    expect(drawPets(drawn)).toEqual(['moon', 'cat', 'star']);
    const found = pets.listPets([{ folder: drawn, origin: 'builtin' }]);
    expect(found.map((pet: { key: string; name: string }) => [pet.key, pet.name])).toEqual([
      ['builtin/cat', 'Little Cat'], ['builtin/moon', 'Little Moon'], ['builtin/star', 'Little Star'],
    ]);
    for (const pet of found) expect(Object.keys(pet.animations ?? {})).toEqual(pets.ROWS);
  });

  it('are looked at a hundred at most in a folder', () => {
    const many = scratch();
    for (let n = 0; n < 101; n++) pack(many, `p${String(n).padStart(3, '0')}`, manifest());
    expect(pets.listPets([{ folder: many, origin: 'user' }])).toHaveLength(100);
  });
});

describe('the picture of a pet', () => {
  it('is handed out as an address of data of its kind', () => {
    const folder = pack(scratch(), 'mochi', manifest());
    const pet = pets.readPet(folder, 'user');
    expect(pets.sheetOf(pet)).toBe(`data:image/png;base64,${png(1536, 1872).toString('base64')}`);
  });

  it('is refused where it changed into something else since it was read', () => {
    const folder = pack(scratch(), 'mochi', manifest());
    const pet = pets.readPet(folder, 'user');
    writeFileSync(pet.sheet, 'now it is words');
    expect(() => pets.sheetOf(pet)).toThrow('no PNG or WebP');
    writeFileSync(pet.sheet, Buffer.alloc(pets.SHEET_MOST + 1));
    expect(() => pets.sheetOf(pet)).toThrow('too large');
  });
});

describe('what the task does', () => {
  it('is passed on as a state that there is, with a short label and bubble on one line', () => {
    expect(pets.shownOf({ state: 'review', label: 'read\nfile', bubble: 'b', more: 1 })).toEqual({ state: 'review', label: 'read file', bubble: 'b' });
    expect(pets.shownOf({ state: 'idle' })).toEqual({ state: 'idle', label: '', bubble: '' });
    expect(pets.shownOf({ state: 'done', label: 'x'.repeat(100), bubble: 'y'.repeat(200) }))
      .toEqual({ state: 'done', label: 'x'.repeat(80), bubble: 'y'.repeat(160) });
    for (const given of [null, 'idle', [], { state: 'sleeping' }, {}]) expect(pets.shownOf(given)).toBeNull();
  });

  it('is written whole for other programs, and taken away again', () => {
    const file = path.join(scratch(), 'deeper', 'pet-state.json');
    expect(pets.writeState(file, { state: 'waiting', label: 'shell', bubble: 'never written' }, 42, 1000)).toBe(true);
    expect(JSON.parse(readFileSync(file, 'utf8'))).toEqual({ version: 1, state: 'waiting', label: 'shell', at: 1000, pid: 42 });
    expect(existsSync(`${file}.tmp`)).toBe(false);
    pets.forgetState(file);
    expect(existsSync(file)).toBe(false);
    // Taking away what is not there is nothing.
    pets.forgetState(file);
  });

  it('leaves what was written before where it cannot be written whole', () => {
    const file = path.join(scratch(), 'pet-state.json');
    pets.writeState(file, { state: 'thinking', label: '', bubble: '' }, 1, 1);
    mkdirSync(`${file}.tmp`);
    expect(pets.writeState(file, { state: 'done', label: '', bubble: '' }, 1, 2)).toBe(false);
    expect(JSON.parse(readFileSync(file, 'utf8')).state).toBe('thinking');
  });

  it('is let go where it cannot be written', () => {
    const parent = scratch();
    writeFileSync(path.join(parent, 'a file'), 'in the way');
    expect(pets.writeState(path.join(parent, 'a file', 'pet-state.json'), { state: 'idle', label: '', bubble: '' }, 1, 1)).toBe(false);
  });
});
