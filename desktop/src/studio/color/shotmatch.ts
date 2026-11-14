/**
 * Shot Match from frame statistics, as the agent's video/shotmatch.py. Resolve does not publish
 * its solver; this uses the same bounded least-squares fit in both clients (RM20, pp. 3047–3049).
 * The shared reference vectors hold the two implementations to the same results.
 */
export const LEVELS = [1, 5, 25, 50, 75, 95, 99] as const;
const CHANNELS = ['r', 'g', 'b'] as const;
const LUMA = [0.2126, 0.7152, 0.0722];
const WEIGHTS = [0.03, 0.12, 0.225, 0.25, 0.225, 0.12, 0.03];
const NAMES = ['gamma.r', 'gamma.g', 'gamma.b', 'gain.r', 'gain.g', 'gain.b', 'offset.r', 'offset.g', 'offset.b', 'contrast'];

export interface Figures { levels: number[][]; means: number[] }
export interface Numbers { r: number; g: number; b: number }
export interface Match { gamma: Numbers; gain: Numbers; offset: Numbers; contrast: number; pivot: number }
export interface Solved { match: Match; before: number; after: number; limited: string[] }

const clamp = (x: number, low: number, high: number) => Math.min(Math.max(x, low), high);
const sum = (values: readonly number[]) => values.reduce((a, b) => a + b, 0);
const dot = (one: readonly number[], other: readonly number[]) => sum(one.map((x, i) => x * other[i]));
const grey = (frame: Figures) => dot(LUMA, frame.levels.map(one => one[3]));
const chroma = (rgb: readonly number[]) => {
  const y = dot(LUMA, rgb);
  return [(rgb[2] - y) / 1.8556, (rgb[0] - y) / 1.5748];
};
const expected = (levels: number[]) => dot(WEIGHTS, levels);
const clipped = (value: number) => value <= 0.002 || value >= 1 - 0.002;
export const neutralMatch = (): Match => ({ gamma: { r: 1, g: 1, b: 1 }, gain: { r: 1, g: 1, b: 1 }, offset: { r: 0, g: 0, b: 0 }, contrast: 1, pivot: 0.435 });
const neutral = (match: Match) => CHANNELS.every(ch => match.gamma[ch] === 1 && match.gain[ch] === 1 && match.offset[ch] === 0) && match.contrast === 1;

export function percentile(counts: readonly number[], share: number): number {
  const total = sum(counts);
  if (counts.length < 2 || total <= 0) return 0;
  const wanted = clamp(share, 0, 1) * total;
  let below = 0;
  for (let bin = 0; bin < counts.length; bin++) {
    const count = counts[bin];
    if (count > 0 && below + count >= wanted) return clamp((bin - 0.5 + (wanted - below) / count) / (counts.length - 1), 0, 1);
    below += count;
  }
  return 1;
}

export function figures(histogram: { red: number[]; green: number[]; blue: number[] }, means: number[]): Figures {
  return { levels: [histogram.red, histogram.green, histogram.blue].map(counts => LEVELS.map(level => percentile(counts, level / 100))), means: [...means] };
}

function channel(x: number, match: Match, at: number): number {
  const ch = CHANNELS[at];
  const exponent = match.gamma[ch];
  const power = exponent === 1 || x === 0 ? x : Math.sign(x) * Math.abs(x) ** exponent;
  const moved = match.gain[ch] * power + match.offset[ch];
  const { contrast, pivot } = match;
  if (contrast === 1 || moved <= 0 || moved >= 1) return moved;
  return moved < pivot ? pivot * (moved / pivot) ** contrast
    : 1 - (1 - pivot) * ((1 - moved) / (1 - pivot)) ** contrast;
}

function undone(y: number, match: Match, at: number): number {
  if (match.gain[CHANNELS[at]] <= 0 || match.contrast <= 0) return y;
  let low = -4; let high = 4;
  if (!(channel(low, match, at) <= y && y <= channel(high, match, at))) return y;
  for (let i = 0; i < 80; i++) {
    const middle = (low + high) / 2;
    if (channel(middle, match, at) < y) low = middle;
    else high = middle;
  }
  return (low + high) / 2;
}

function moved(frame: Figures, by: (x: number, at: number) => number): Figures {
  const levels = frame.levels.map((one, at) => one.map(x => clamp(by(x, at), 0, 1)));
  return { levels, means: frame.means.map((mean, at) => mean + expected(levels[at]) - expected(frame.levels[at])) };
}
export const mapped = (frame: Figures, match: Match): Figures => moved(frame, (x, at) => channel(x, match, at));
export const unmapped = (frame: Figures, match: Match): Figures => moved(frame, (x, at) => undone(x, match, at));

function rms(one: Figures, other: Figures, kept: Figures): number {
  const squares = one.levels.flatMap((xs, at) => xs.flatMap((x, i) => {
    const target = other.levels[at][i];
    return clipped(target) || clipped(kept.levels[at][i]) ? [] : [(x - target) ** 2];
  }));
  return squares.length ? Math.sqrt(sum(squares) / squares.length) : 0;
}

export function solve(clip: Figures, target: Figures): Solved {
  const pivot = clamp(grey(target), 0.18, 0.82);
  const lows = [...Array<number>(6).fill(Math.log(0.5)), -0.2, -0.2, -0.2, Math.log(2 / 3)];
  const highs = [...Array<number>(6).fill(Math.log(2)), 0.2, 0.2, 0.2, Math.log(1.5)];
  const spans = highs.map((high, i) => (high - lows[i]) / 2);
  const weights = clip.levels.map((one, at) => one.map((x, i) => clipped(x) || clipped(target.levels[at][i]) ? 0.2 : 1));
  const wantedChroma = chroma(target.means);
  const matchOf = (theta: number[]): Match => ({
    gamma: { r: Math.exp(theta[0]), g: Math.exp(theta[1]), b: Math.exp(theta[2]) },
    gain: { r: Math.exp(theta[3]), g: Math.exp(theta[4]), b: Math.exp(theta[5]) },
    offset: { r: theta[6], g: theta[7], b: theta[8] }, contrast: Math.exp(theta[9]), pivot,
  });
  const residuals = (theta: number[]): number[] => {
    const match = matchOf(theta);
    const out: number[] = []; const medians: number[] = []; const means: number[] = [];
    for (let at = 0; at < 3; at++) {
      const moved = clip.levels[at].map(x => channel(x, match, at));
      out.push(...moved.map((x, i) => weights[at][i] * (x - target.levels[at][i])));
      medians.push(moved[3]);
      means.push(clip.means[at] + expected(moved) - expected(clip.levels[at]));
    }
    const [cb, cr] = chroma(means);
    out.push(2 * (cb - wantedChroma[0]), 2 * (cr - wantedChroma[1]), 2 * (dot(LUMA, medians) - grey(target)));
    out.push(...theta.map((x, i) => 1e-2 * x / spans[i]));
    return out;
  };
  const theta = leastSquares(residuals, Array<number>(10).fill(0), lows, highs).map(x => Math.abs(x) < 1e-4 ? 0 : x);
  const match = matchOf(theta);
  if (neutral(match)) match.pivot = 0.435;
  return {
    match, before: rms(clip, target, clip), after: rms(mapped(clip, match), target, clip),
    limited: NAMES.filter((_, i) => theta[i] <= lows[i] + 1e-9 || theta[i] >= highs[i] - 1e-9),
  };
}

function leastSquares(residuals: (theta: number[]) => number[], start: number[], lows: number[], highs: number[]): number[] {
  let theta = [...start]; let now = residuals(theta); let cost = dot(now, now); let damping = 1e-3;
  for (let iteration = 0; iteration < 200; iteration++) {
    const columns = theta.map((value, i) => {
      const up = [...theta]; const down = [...theta];
      up[i] = Math.min(value + 1e-6, highs[i]); down[i] = Math.max(value - 1e-6, lows[i]);
      const width = up[i] - down[i];
      const upper = residuals(up); const lower = residuals(down);
      return upper.map((x, j) => (x - lower[j]) / width);
    });
    const gradient = columns.map(column => dot(column, now));
    const normal = columns.map(one => columns.map(other => dot(one, other)));
    const free = theta.map((_, i) => i).filter(i => !(theta[i] <= lows[i] && gradient[i] > 0) && !(theta[i] >= highs[i] && gradient[i] < 0));
    if (free.length === 0 || Math.max(...free.map(i => Math.abs(gradient[i]))) < 1e-14) break;
    let improved = false;
    while (damping < 1e12) {
      const system = free.map(i => free.map(k => normal[i][k] + (i === k ? damping * (normal[i][i] + 1e-12) : 0)));
      const step = linearSolve(system, free.map(i => -gradient[i]));
      const trial = [...theta];
      free.forEach((i, at) => { trial[i] = clamp(theta[i] + step[at], lows[i], highs[i]); });
      const after = residuals(trial); const trialCost = dot(after, after);
      if (trialCost < cost) {
        const gained = cost - trialCost;
        theta = trial; now = after; cost = trialCost;
        damping = Math.max(damping / 3, 1e-12);
        improved = gained > 1e-16 * Math.max(cost, 1e-30) && gained > 1e-20;
        break;
      }
      damping *= 4;
    }
    if (!improved) break;
  }
  return theta;
}

/** Gaussian elimination with partial pivoting, the same ten-parameter system as Python. */
function linearSolve(matrix: number[][], vector: number[]): number[] {
  const size = vector.length;
  const rows = matrix.map((row, i) => [...row, vector[i]]);
  for (let column = 0; column < size; column++) {
    let best = column;
    for (let row = column + 1; row < size; row++) if (Math.abs(rows[row][column]) > Math.abs(rows[best][column])) best = row;
    [rows[column], rows[best]] = [rows[best], rows[column]];
    const lead = rows[column][column];
    if (Math.abs(lead) < 1e-300) continue;
    for (let row = column + 1; row < size; row++) {
      const factor = rows[row][column] / lead;
      if (factor !== 0) for (let k = column; k <= size; k++) rows[row][k] -= factor * rows[column][k];
    }
  }
  const out = Array<number>(size).fill(0);
  for (let row = size - 1; row >= 0; row--) {
    const lead = rows[row][row];
    let rest = 0;
    for (let k = row + 1; k < size; k++) rest += rows[row][k] * out[k];
    out[row] = Math.abs(lead) >= 1e-300 ? (rows[row][size] - rest) / lead : 0;
  }
  return out;
}
