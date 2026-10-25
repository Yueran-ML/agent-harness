// Draw the pets that come with the application, each as a package of a pet: `pet.json` and a sheet
// of its frames, in `<folder>/<id>/`. They are pixel art, drawn here from shapes, so that the
// application brings no picture that somebody else made, and the repository no picture at all:
// the preparation of the assets draws them into `public/pets/` for every build.
//
// Usage: node scripts/draw-pets.mjs [folder]   (public/pets of the desktop folder by default)
//
// The sheet is laid out as the pets of Codex are: cells of 192 by 208 pixels, eight to a row, a row
// for each of nine ways of being. See docs/pets.md in the repository for the format.
import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateSync } from 'node:zlib';

const here = path.dirname(fileURLToPath(import.meta.url));

// A frame is drawn at a quarter of its size and made four times larger, pixel for pixel.
const SCALE = 4;
const W = 48;
const H = 52;
const COLUMNS = 8;
// The rows of the sheet, in the order of the format, with how many frames each has and how fast they go.
export const ROWS = [
  ['idle', 6, 5], ['runRight', 8, 10], ['runLeft', 8, 10], ['wave', 4, 6], ['jump', 5, 8],
  ['failed', 8, 6], ['waiting', 6, 4], ['thinking', 6, 5], ['review', 6, 4],
];
const OUTLINE = [30, 26, 46, 255];

/** A frame at a quarter of its size: its pixels, and the shapes that are drawn on it. */
function frame() {
  const pixels = new Array(W * H).fill(null);
  const set = (x, y, colour) => {
    const [px, py] = [Math.round(x), Math.round(y)];
    if (px >= 0 && py >= 0 && px < W && py < H) pixels[py * W + px] = colour;
  };
  return {
    pixels,
    set,
    get: (x, y) => (x >= 0 && y >= 0 && x < W && y < H ? pixels[y * W + x] : null),
    disc(cx, cy, rx, ry, colour) {
      for (let y = Math.floor(cy - ry); y <= Math.ceil(cy + ry); y++) {
        for (let x = Math.floor(cx - rx); x <= Math.ceil(cx + rx); x++) {
          if (((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2 <= 1) set(x, y, colour);
        }
      }
    },
    rect(x0, y0, x1, y1, colour) {
      for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) set(x, y, colour);
    },
    polygon(points, colour) {
      const ys = points.map(([, y]) => y);
      for (let y = Math.floor(Math.min(...ys)); y <= Math.ceil(Math.max(...ys)); y++) {
        for (let x = 0; x < W; x++) if (inside(points, x, y)) set(x, y, colour);
      }
    },
    line(x0, y0, x1, y1, colour) {
      const steps = Math.max(Math.abs(x1 - x0), Math.abs(y1 - y0), 1);
      for (let i = 0; i <= steps; i++) set(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps, colour);
    },
    /** Put a dark line around everything that is drawn, as pixel art has. */
    outline() {
      const before = pixels.slice();
      for (let y = 0; y < H; y++) {
        for (let x = 0; x < W; x++) {
          if (before[y * W + x]) continue;
          const near = [[1, 0], [-1, 0], [0, 1], [0, -1]].some(([dx, dy]) => {
            const [nx, ny] = [x + dx, y + dy];
            return nx >= 0 && ny >= 0 && nx < W && ny < H && before[ny * W + nx] && before[ny * W + nx] !== OUTLINE;
          });
          if (near) pixels[y * W + x] = OUTLINE;
        }
      }
    },
  };
}

function inside(points, x, y) {
  let within = false;
  for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
    const [xi, yi] = points[i];
    const [xj, yj] = points[j];
    if ((yi > y) !== (yj > y) && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) within = !within;
  }
  return within;
}

const hex = (text) => [1, 3, 5].map((at) => Number.parseInt(text.slice(at, at + 2), 16)).concat(255);
const INK = hex('#2b2340');
const BLUSH = hex('#f2a0b4');
const WHITE = hex('#ffffff');
const PAPER = hex('#fbf6ea');
const COVER = hex('#7b5ea7');
const SWEAT = hex('#8fd0ff');
const SPARK = hex('#fff2a8');
const CLOUD = hex('#f4f2ff');

/**
 * How a pet stands in one frame: moved, squashed, how its eyes and mouth are, whether a hand is up,
 * and what is drawn beside it.
 */
function poseOf(row, i) {
  const pose = { dx: 0, dy: 0, squash: 0, eyes: 'open', mouth: 'smile', hand: null, extras: [], lean: 0 };
  switch (row) {
    case 'idle':
      pose.dy = [0, 0, -1, -1, 0, 0][i];
      if (i === 4) pose.eyes = 'closed';
      break;
    case 'runRight':
    case 'runLeft':
      pose.dy = [0, -2, -3, -2, 0, -2, -3, -2][i];
      pose.dx = [-2, -1, 0, 1, 2, 1, 0, -1][i];
      pose.lean = 1;
      pose.feet = i % 2;
      pose.mouth = 'open';
      break;
    case 'wave':
      pose.hand = [[-3], [-6], [-3], [-6]][i][0];
      pose.mouth = 'open';
      if (i % 2 === 1) pose.extras.push(['spark', 38, 12]);
      break;
    case 'jump':
      pose.dy = [0, -5, -9, -5, 0][i];
      pose.squash = [2, 0, -1, 0, 2][i];
      pose.mouth = 'open';
      if (i === 2) pose.extras.push(['spark', 10, 14], ['spark', 38, 10]);
      break;
    case 'failed':
      pose.dx = [-1, 1, -1, 1, 0, 0, 0, 0][i];
      pose.eyes = 'sad';
      pose.mouth = 'frown';
      pose.extras.push(['sweat', 35, 17 + (i % 4)]);
      break;
    case 'waiting':
      pose.dy = [0, 0, 0, -1, -1, 0][i];
      if (i === 3) pose.eyes = 'closed';
      pose.mouth = 'flat';
      pose.extras.push(['dots', Math.min(3, Math.floor(i / 2) + 1)]);
      break;
    case 'thinking':
      pose.eyes = 'up';
      pose.mouth = 'flat';
      pose.extras.push(['thought', i % 3]);
      break;
    case 'review':
      pose.eyes = 'down';
      pose.mouth = 'flat';
      pose.extras.push(['book', i % 3]);
      break;
  }
  return pose;
}

/** Draw what is beside a pet, after the pet and its line, so that the two do not run together. */
function extras(f, pose, top) {
  for (const [kind, ...at] of pose.extras) {
    if (kind === 'spark') {
      const [x, y] = at;
      f.set(x, y, SPARK); f.set(x - 1, y, SPARK); f.set(x + 1, y, SPARK); f.set(x, y - 1, SPARK); f.set(x, y + 1, SPARK);
    } else if (kind === 'sweat') {
      const [x, y] = at;
      f.disc(x, y, 1.2, 1.6, SWEAT); f.set(x, y - 2, SWEAT);
    } else if (kind === 'dots') {
      for (let n = 0; n < at[0]; n++) f.rect(18 + n * 5, top - 4, 19 + n * 5, top - 3, CLOUD);
    } else if (kind === 'thought') {
      f.disc(37, 9, 6, 4, CLOUD); f.disc(32, 15, 1.5, 1.5, CLOUD); f.disc(30, 18, 1, 1, CLOUD);
      for (let n = 0; n < 3; n++) f.set(34 + n * 3, 9, n === at[0] ? COVER : INK);
    } else if (kind === 'book') {
      f.rect(15, 35, 33, 43, COVER);
      f.rect(16, 36, 23, 42, PAPER);
      f.rect(25, 36, 32, 42, PAPER);
      for (let n = 0; n < 3; n++) { f.line(17, 38 + n * 2, 22, 38 + n * 2, INK); f.line(26, 38 + n * 2, 31, 38 + n * 2, INK); }
      // A page turns over the middle.
      if (at[0] === 1) f.rect(24, 34, 28, 41, PAPER);
      if (at[0] === 2) f.rect(20, 34, 24, 41, PAPER);
    }
  }
}

/** A face: two eyes as the pose says, a mouth, and cheeks, around the middle of the face. */
function face(f, cx, cy, pose, eyeGap = 4, colour = INK) {
  for (const side of [-1, 1]) {
    const x = cx + side * eyeGap;
    if (pose.eyes === 'closed') f.line(x - 1, cy, x + 1, cy, colour);
    else if (pose.eyes === 'sad') { f.set(x - side, cy - 1, colour); f.set(x, cy, colour); f.set(x - side, cy + 1, colour); }
    else if (pose.eyes === 'up') { f.rect(x, cy - 1, x, cy, colour); f.set(x, cy - 2, WHITE); }
    else if (pose.eyes === 'down') { f.rect(x, cy, x, cy + 1, colour); }
    else { f.rect(x, cy - 1, x, cy, colour); }
    f.set(x + side * 2, cy + 2, BLUSH);
  }
  const my = cy + 3;
  if (pose.mouth === 'open') { f.rect(cx - 1, my, cx, my + 1, colour); }
  else if (pose.mouth === 'frown') { f.set(cx - 1, my + 1, colour); f.set(cx, my, colour); f.set(cx + 1, my + 1, colour); }
  else if (pose.mouth === 'flat') { f.line(cx - 1, my, cx, my, colour); }
  else { f.set(cx - 1, my, colour); f.set(cx, my + 1, colour); f.set(cx + 1, my, colour); }
}

/** The little moon: a round body lit from the left, with a star above it. */
function moon(f, pose) {
  const cx = 24 + pose.dx + pose.lean;
  const cy = 30 + pose.dy;
  const light = hex('#f8eab0');
  const shade = hex('#e2c46e');
  const feet = hex('#d9b25a');
  const lift = pose.feet === 1 ? -1 : 0;
  f.disc(cx - 5, 44 + pose.dy * 0.3 + lift, 3, 2, feet);
  f.disc(cx + 5, 44 + pose.dy * 0.3 - lift, 3, 2, feet);
  f.disc(cx, cy, 13 + pose.squash * 0.5, 13 - pose.squash, light);
  // The shadow of a moon that is not full, on its right.
  for (let y = cy - 13; y <= cy + 13; y++) {
    for (let x = cx; x <= cx + 13; x++) {
      const onBody = ((x - cx) / (13 + pose.squash * 0.5)) ** 2 + ((y - cy) / (13 - pose.squash)) ** 2 <= 1;
      const outsideLight = ((x - cx + 6) / 15) ** 2 + ((y - cy) / 15) ** 2 > 1;
      if (onBody && outsideLight) f.set(x, y, shade);
    }
  }
  f.disc(cx - 6, cy - 6, 1.5, 1.5, hex('#efd78a'));
  if (pose.hand !== null) f.disc(cx + 14, cy + pose.hand, 2, 2, light);
  const top = cy - 15;
  const star = [cx, top - 2];
  f.set(star[0], star[1], SPARK); f.set(star[0] - 1, star[1], SPARK); f.set(star[0] + 1, star[1], SPARK);
  f.set(star[0], star[1] - 1, SPARK); f.set(star[0], star[1] + 1, SPARK);
  f.outline();
  face(f, cx - 1, cy, pose);
  return top - 3;
}

/** The little cat: a navy cat that sits, with a tail that curls. */
function cat(f, pose) {
  const cx = 24 + pose.dx + pose.lean;
  const cy = 24 + pose.dy;
  const fur = hex('#3d4373');
  const belly = hex('#6a70a8');
  const pink = hex('#f3a6c0');
  const lift = pose.feet === 1 ? -1 : 0;
  // The tail, behind the body.
  const sway = pose.feet === undefined ? [0, 1, 1, 0, -1, -1][(pose.dy + 2) % 6] ?? 0 : pose.feet;
  for (let t = 0; t <= 10; t++) {
    const x = cx + 10 + Math.sin(t / 3) * 3 + sway;
    const y = 42 + pose.dy * 0.4 - t;
    f.disc(x, y, 1.6, 1.6, fur);
  }
  f.disc(cx, 37 + pose.dy * 0.4, 10 + pose.squash * 0.5, 8 - pose.squash * 0.5, fur);
  f.disc(cx, 39 + pose.dy * 0.4, 5, 5, belly);
  f.disc(cx - 5, 45 + pose.dy * 0.3 + lift, 2.5, 1.5, fur);
  f.disc(cx + 5, 45 + pose.dy * 0.3 - lift, 2.5, 1.5, fur);
  f.disc(cx, cy, 10, 8.5 - pose.squash * 0.3, fur);
  f.polygon([[cx - 9, cy - 3], [cx - 8, cy - 13], [cx - 2, cy - 7]], fur);
  f.polygon([[cx + 9, cy - 3], [cx + 8, cy - 13], [cx + 2, cy - 7]], fur);
  f.polygon([[cx - 7, cy - 5], [cx - 7, cy - 10], [cx - 4, cy - 7]], pink);
  f.polygon([[cx + 7, cy - 5], [cx + 7, cy - 10], [cx + 4, cy - 7]], pink);
  if (pose.hand !== null) f.disc(cx + 11, cy + 10 + pose.hand, 2, 2, fur);
  f.outline();
  // Eyes that shine in the dark fur, and a pink nose.
  const eyes = hex('#f7d774');
  for (const side of [-1, 1]) {
    const x = cx + side * 4;
    if (pose.eyes === 'closed') f.line(x - 1, cy, x + 1, cy, eyes);
    else if (pose.eyes === 'sad') { f.set(x - side, cy - 1, eyes); f.set(x, cy, eyes); f.set(x - side, cy + 1, eyes); }
    else {
      f.rect(x - 1, cy - 1, x, cy, eyes);
      const look = pose.eyes === 'up' ? -1 : pose.eyes === 'down' ? 0 : 0;
      f.set(x - (pose.eyes === 'down' ? 0 : 1), cy + look, INK);
    }
  }
  f.set(cx, cy + 2, pink);
  if (pose.mouth === 'open') f.rect(cx - 1, cy + 3, cx, cy + 4, pink);
  else if (pose.mouth === 'frown') { f.set(cx - 1, cy + 4, belly); f.set(cx, cy + 3, belly); f.set(cx + 1, cy + 4, belly); }
  else { f.set(cx - 1, cy + 3, belly); f.set(cx + 1, cy + 3, belly); }
  return cy - 14;
}

/** The little star spirit: a star that floats, lit in its middle. */
function star(f, pose) {
  const cx = 24 + pose.dx + pose.lean;
  const float = pose.feet === undefined ? 0 : pose.feet;
  const cy = 29 + pose.dy - float;
  const outer = 15 + pose.squash * 0.5;
  const inner = 7;
  const points = [];
  for (let n = 0; n < 10; n++) {
    const angle = -Math.PI / 2 + (n * Math.PI) / 5;
    const r = n % 2 === 0 ? outer : inner;
    points.push([cx + Math.cos(angle) * r, cy + Math.sin(angle) * r * (1 - pose.squash * 0.04)]);
  }
  f.polygon(points, hex('#8ecbff'));
  f.disc(cx, cy + 1, 6, 5, hex('#d8f0ff'));
  if (pose.hand !== null) f.disc(cx + 13, cy + 2 + pose.hand, 2, 2, hex('#8ecbff'));
  f.outline();
  // Its glow below it, as it floats, without a line around it.
  f.rect(cx - 4, 46, cx + 4, 46, hex('#5f86b8'));
  face(f, cx, cy + 1, pose, 3);
  return cy - outer - 2;
}

const PETS = [
  { id: 'moon', displayName: 'Little Moon', description: 'A small moon that keeps you company.', draw: moon },
  { id: 'cat', displayName: 'Little Cat', description: 'A navy cat that watches the work.', draw: cat },
  { id: 'star', displayName: 'Little Star', description: 'A star spirit that floats beside the task.', draw: star },
];

/** The sheet of a pet: every frame, made four times larger, in its cell. */
function sheetOf(draw) {
  const width = COLUMNS * W * SCALE;
  const height = ROWS.length * H * SCALE;
  const data = Buffer.alloc(width * height * 4);
  ROWS.forEach(([row, frames], r) => {
    for (let i = 0; i < frames; i++) {
      const f = frame();
      const pose = poseOf(row, i);
      const top = draw(f, pose);
      extras(f, pose, top);
      const mirrored = row === 'runLeft';
      for (let y = 0; y < H; y++) {
        for (let x = 0; x < W; x++) {
          const colour = f.pixels[y * W + (mirrored ? W - 1 - x : x)];
          if (!colour) continue;
          for (let sy = 0; sy < SCALE; sy++) {
            for (let sx = 0; sx < SCALE; sx++) {
              const px = (i * W + x) * SCALE + sx;
              const py = (r * H + y) * SCALE + sy;
              data.set(colour, (py * width + px) * 4);
            }
          }
        }
      }
    }
  });
  return { width, height, data };
}

const CRC = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(bytes) {
  let c = 0xffffffff;
  for (const byte of bytes) c = CRC[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, body) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(body.length, 0);
  head.write(type, 4, 'ascii');
  const tail = Buffer.alloc(4);
  tail.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), body])), 0);
  return Buffer.concat([head, body, tail]);
}
/** A picture as a PNG: eight bits for each of red, green, blue and how much shows. */
function png({ width, height, data }) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 6, 0, 0, 0], 8);
  const rows = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) data.copy(rows, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4);
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header), chunk('IDAT', deflateSync(rows, { level: 9 })), chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** Draw every pet into a folder of its own in `out`, and return their ids. */
export function drawPets(out) {
  for (const pet of PETS) {
    const folder = path.join(out, pet.id);
    mkdirSync(folder, { recursive: true });
    writeFileSync(path.join(folder, 'spritesheet.png'), png(sheetOf(pet.draw)));
    const animations = Object.fromEntries(ROWS.map(([name, frames, fps], row) => [name, { row, frames, fps }]));
    const manifest = {
      id: pet.id, displayName: pet.displayName, description: pet.description, spritesheetPath: 'spritesheet.png', animations,
    };
    writeFileSync(path.join(folder, 'pet.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  }
  return PETS.map((pet) => pet.id);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const out = path.resolve(process.argv[2] ?? path.join(here, '..', 'public', 'pets'));
  console.log(`Drew ${drawPets(out).join(', ')} in ${out}.`);
}
