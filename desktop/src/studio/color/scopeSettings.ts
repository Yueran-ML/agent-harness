/**
 * The video scopes of the Color page as DaVinci Resolve has them (Reference Manual 20,
 * pp. 3007–3022; the interaction reference, §6): what each pane of the scopes is set to, the
 * options of the menu that hold for every scope, how those become the settings of `scopes.set`
 * (PROTOCOL.md, "Scopes"), how large a picture is asked for, and how the settings are kept in the
 * preferences of this desktop. Pure functions: the panel is in `Scopes.tsx`, the subscriptions in
 * `useScope.ts`.
 *
 * The graticule is drawn by the page, not into the picture (`look.graticule` is 0). The picture
 * is a grid of counts and is scaled to its pane, while the lines and their labels are drawn over
 * it in pixels of the screen: one pixel wide and sharp at any size and on a screen of high
 * density, the labels in the type of the page (the engine draws no text), and the Graticule
 * slider and the Show Graticule boxes change them at once, with nothing asked of the engine.
 */
import type { ScopeGraticule } from '../engine';

/** The five scopes, in DaVinci Resolve's order (p. 3013). */
export const SCOPE_KINDS = ['waveform', 'parade', 'vectorscope', 'histogram', 'chromaticity'] as const;
export type ScopeKind = typeof SCOPE_KINDS[number];

/** How many panes the floating window shows (p. 3018): 1, 2, 4 or 9. */
export const LAYOUTS = [1, 2, 4, 9] as const;
export type Layout = typeof LAYOUTS[number];
/** The panes of the floating window, of which a layout shows the first. */
export const PANES = 9;

/** The gamuts that a chromaticity diagram can draw a second triangle of (p. 3022), by the engine's names. */
export const GAMUTS = ['rec709', 'rec2020', 'dci-p3', 'p3-d65', 'rec601-pal', 'rec601-ntsc'] as const;
export type Gamut = typeof GAMUTS[number];
/** How the gamuts are named, as DaVinci Resolve names them in every language. */
export const GAMUT_NAMES: Record<Gamut, string> = {
  rec709: 'Rec.709', rec2020: 'Rec.2020', 'dci-p3': 'DCI-P3', 'p3-d65': 'P3-D65', 'rec601-pal': 'Rec.601 PAL', 'rec601-ntsc': 'Rec.601 NTSC',
};

/** The Show Reference Levels box and its Low and High sliders, in codes of 10 bits (pp. 3019–3021). */
export interface ReferenceLevels { shown: boolean; low: number; high: number }
/** Where the reference levels rest; a line is drawn only for one moved off it (p. 3019). */
export const REFERENCE_REST = { low: 0, high: 1023 } as const;

/**
 * The Custom Controls of each scope (pp. 3019–3022). `brightness` is the slider named after the
 * scope (Gain on the histogram, which makes its graph taller, p. 3021), and `graticule` the
 * Graticule slider, both 0–1. What the engine does not draw yet is not here: Colorize, Extents,
 * Combine, the CbCr mode, the R, G and B buttons, YRGB and Y'CbCr parades, and CIE 1976 uv.
 */
export interface WaveformControls { channels: 'luma' | 'rgb'; brightness: number; graticule: number; reference: ReferenceLevels }
export interface ParadeControls { brightness: number; graticule: number; reference: ReferenceLevels }
export interface VectorscopeControls {
  tones: 'all' | 'low' | 'mid' | 'high';
  /** Low Range and High Range, 0.30 and 0.70 (p. 3021). */
  low: number; high: number;
  zoom2x: boolean; skinTone: boolean; showGraticule: boolean;
  brightness: number; graticule: number;
}
export interface HistogramControls { channels: 'rgb' | 'yrgb'; brightness: number; graticule: number; reference: ReferenceLevels }
export interface ChromaticityControls { extra: Gamut | null; showGraticule: boolean; brightness: number; graticule: number }

/** A pane of the scopes: the scope it shows, and the controls of each scope, kept as the pane goes from one to another. */
export interface PaneSettings {
  kind: ScopeKind;
  waveform: WaveformControls;
  parade: ParadeControls;
  vectorscope: VectorscopeControls;
  histogram: HistogramControls;
  chromaticity: ChromaticityControls;
}

/** The option menu of the scopes (…), which holds for all of them (pp. 3009–3010). */
export interface ScopeOptions {
  /** Waveform Scale Style: 10-bit (the default), Percentage or HDR (ST.2084/HLG); 12-bit and mV are not done. */
  scale: '10-bit' | 'percent' | 'nits';
  /** Vectorscope Scale Style: Standard (the default) or Off; Simplified and Hue Vectors are not done. */
  vectorStyle: 'standard' | 'off';
  /** The targets of the bars: 75% (the traditional ones, p. 3015), 100% or both. */
  targets: '75' | '100' | 'both';
  /** Whether the hues of the targets are named. */
  hueLabels: boolean;
}

/** All that is kept of the scopes: the options, the docked pane, and the floating window. */
export interface ScopesSettings {
  options: ScopeOptions;
  docked: PaneSettings;
  floating: { layout: Layout; panes: PaneSettings[]; rect: FloatRect | null };
}
/** Where the floating window lies on the page, in its pixels. */
export interface FloatRect { x: number; y: number; width: number; height: number }

const rest = (): ReferenceLevels => ({ shown: false, ...REFERENCE_REST });

/**
 * A pane as it is first shown. The manual gives no defaults but Low and High Range (p. 3021), so
 * these are ours (the interaction reference, §6.3): the waveform in RGB, the histogram in RGB, the
 * vectorscope over all tones without the skin tone line or the 2X zoom, the sliders at the middle.
 */
export function newPane(kind: ScopeKind = 'parade'): PaneSettings {
  return {
    kind,
    waveform: { channels: 'rgb', brightness: 0.5, graticule: 0.5, reference: rest() },
    parade: { brightness: 0.5, graticule: 0.5, reference: rest() },
    vectorscope: { tones: 'all', low: 0.3, high: 0.7, zoom2x: false, skinTone: false, showGraticule: true, brightness: 0.5, graticule: 0.5 },
    histogram: { channels: 'rgb', brightness: 0.5, graticule: 0.5, reference: rest() },
    chromaticity: { extra: null, showGraticule: true, brightness: 0.5, graticule: 0.5 },
  };
}
/** What the panes of the floating window show at first, so that 4-Up shows the four scopes most used together. */
const FLOATING_KINDS: readonly ScopeKind[] = ['parade', 'waveform', 'vectorscope', 'histogram', 'chromaticity', 'waveform', 'parade', 'vectorscope', 'histogram'];
export const DEFAULT_OPTIONS: ScopeOptions = { scale: '10-bit', vectorStyle: 'standard', targets: '75', hueLabels: true };
export function defaultScopes(): ScopesSettings {
  return { options: { ...DEFAULT_OPTIONS }, docked: newPane('parade'), floating: { layout: 4, panes: FLOATING_KINDS.map(newPane), rect: null } };
}

// ---- The settings of the engine ------------------------------------------------------------

/** The size of the picture asked of a scope: the fields of its kind that say it. */
export type PictureSize = Record<string, number>;

/** How finely the size of a picture follows its pane, so that a pane resized a little asks for nothing. */
const STEP = 32;
/** The fewest and the most pixels each way, and in all: the pictures come as base64 in lines of JSON, up to 30 a second. */
const LEAST = 64;
const MOST = 1024;
const MOST_AREA = 1024 * 384;
const down = (value: number) => Math.max(LEAST, Math.min(MOST, Math.floor(value / STEP) * STEP));

/**
 * The size of picture asked of a scope of `kind` shown in a pane of `width` × `height` pixels of
 * the page, on a screen of `ratio` pixels to each of the page: as many counts as the screen has
 * pixels there, in steps of 32, at least 64 and at most 1024 each way and 1024 × 384 in all. The
 * vectorscope is square and the chromaticity diagram is 8 wide to 9 high (PROTOCOL.md, "Scopes").
 */
export function pictureSize(kind: ScopeKind, width: number, height: number, ratio = 1): PictureSize {
  const w0 = Math.max(1, width * ratio);
  const h0 = Math.max(1, height * ratio);
  const shrink = Math.min(1, Math.sqrt(MOST_AREA / (w0 * h0)));
  const [w, h] = [w0 * shrink, h0 * shrink];
  switch (kind) {
    case 'waveform': return { width: down(w), levels: down(h) };
    // Three panels side by side, each a third of the picture.
    case 'parade': return { width: Math.max(LEAST / 2, Math.min(MOST / 2, Math.floor(w / 3 / 16) * 16)), levels: down(h) };
    case 'vectorscope': return { size: down(Math.min(w0, h0, Math.sqrt(MOST_AREA))) };
    case 'histogram': return { bins: down(w), height: down(h) };
    case 'chromaticity': {
      const across = Math.min(w0, (h0 * 8) / 9, Math.sqrt((MOST_AREA * 8) / 9));
      const wide = Math.max(LEAST, Math.min(896, Math.floor(across / STEP) * STEP));
      return { width: wide, height: (wide / 8) * 9 };
    }
  }
}

/**
 * The settings of `scopes.subscribe` and `scopes.set` for a pane (PROTOCOL.md, "Scopes"): its
 * scope and what of its controls and of the options the engine draws, at the size given. The
 * graticule is not drawn into the picture (`look.graticule` 0): the page draws it. The signal is
 * left to the engine, Rec.709 in luma and transfer, as DaVinci YRGB is; the chromaticity diagram
 * is of Rec.709, with a second triangle where one is chosen.
 */
export function engineSettings(pane: PaneSettings, options: ScopeOptions, size: PictureSize): Record<string, unknown> {
  const look = (brightness: number) => ({ brightness: clamp01(brightness), graticule: 0 });
  switch (pane.kind) {
    case 'waveform': {
      const { channels, brightness } = pane.waveform;
      return { kind: 'waveform', ...size, channels, scale: options.scale, look: look(brightness) };
    }
    case 'parade':
      return { kind: 'parade', ...size, scale: options.scale, look: look(pane.parade.brightness) };
    case 'vectorscope': {
      const scope = pane.vectorscope;
      const off = options.vectorStyle === 'off';
      const low = clamp01(Math.min(scope.low, scope.high));
      return {
        kind: 'vectorscope', ...size, zoom: scope.zoom2x ? 2 : 1, tones: scope.tones, low, high: Math.max(low, clamp01(scope.high)),
        targets: off ? 'off' : options.targets, skin_tone: !off && scope.skinTone, look: look(scope.brightness),
      };
    }
    case 'histogram': {
      const { channels, brightness } = pane.histogram;
      return { kind: 'histogram', ...size, channels, scale: options.scale, look: look(brightness) };
    }
    case 'chromaticity': {
      const { extra, brightness } = pane.chromaticity;
      const gamuts = [{ name: GAMUT_NAMES.rec709, primaries: 'rec709' }];
      if (extra !== null && extra !== 'rec709') gamuts.push({ name: GAMUT_NAMES[extra], primaries: extra });
      return { kind: 'chromaticity', ...size, space: 'rec709', gamuts, look: look(brightness) };
    }
  }
}
const clamp01 = (value: number) => (Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0.5);

// ---- What the page draws over the picture --------------------------------------------------

/** A line of the graticule as the page draws it, in pixels of the picture. */
export interface DrawnLine { x1: number; y1: number; x2: number; y2: number; colour: string; dashed?: boolean }
export interface DrawnLabel { x: number; y: number; text: string; colour: string }

/** The colours of the engine for the faint grid of the chromaticity diagram (`graticule.rs`). */
const FAINT = '72,72,72';
/** The names that the engine gives the targets of the vectorscope. */
const HUES = new Set(['R', 'Mg', 'B', 'Cy', 'G', 'Yl']);
/** The colour of the reference levels, as the markers of a scale are drawn in DaVinci Resolve: a warm orange. */
const REFERENCE = '236,140,48';
const rgb = (colour: readonly number[]) => colour.slice(0, 3).join(',');

/**
 * Whether a line of a vectorscope's graticule is of its circle or of the cross at its centre, which
 * Show Graticule shows and hides (p. 3021), and not a target or the skin tone line.
 */
export function isCircleOrCross(line: ScopeGraticule['lines'][number], size: number): boolean {
  const centre = size / 2;
  const radius = centre - 0.5;
  const from = Math.hypot(line.from[0] - centre, line.from[1] - centre);
  const to = Math.hypot(line.to[0] - centre, line.to[1] - centre);
  if (Math.abs(from - radius) < 1 && Math.abs(to - radius) < 1) return true;
  const middle = [(line.from[0] + line.to[0]) / 2, (line.from[1] + line.to[1]) / 2];
  const level = Math.abs(line.from[1] - line.to[1]) < 1e-3 || Math.abs(line.from[0] - line.to[0]) < 1e-3;
  return level && Math.hypot(middle[0] - centre, middle[1] - centre) < 0.5;
}

/**
 * What the page draws over a scope's picture: the graticule the engine gave, less what a box of
 * the pane hides, and the reference levels moved off where they rest (p. 3019), across the
 * waveform and the parade and up the histogram, at the code of 10 bits that each slider says.
 */
export function overlayOf(graticule: ScopeGraticule, pane: PaneSettings, options: ScopeOptions): { lines: DrawnLine[]; labels: DrawnLabel[] } {
  let lines = graticule.lines.map(line => ({ x1: line.from[0], y1: line.from[1], x2: line.to[0], y2: line.to[1], colour: rgb(line.colour), line }));
  let labels = graticule.labels.map(label => ({ x: label.at[0], y: label.at[1], text: label.text, colour: rgb(label.colour) }));
  if (pane.kind === 'vectorscope') {
    const size = graticule.width;
    if (!pane.vectorscope.showGraticule || options.vectorStyle === 'off') lines = lines.filter(each => !isCircleOrCross(each.line, size));
    if (!options.hueLabels || options.vectorStyle === 'off') labels = labels.filter(each => !HUES.has(each.text));
  }
  if (pane.kind === 'chromaticity' && !pane.chromaticity.showGraticule) {
    lines = lines.filter(each => each.colour !== FAINT);
    labels = labels.filter(each => each.colour !== FAINT);
  }
  const drawn: DrawnLine[] = lines.map(({ x1, y1, x2, y2, colour }) => ({ x1, y1, x2, y2, colour }));
  const reference = pane.kind === 'waveform' || pane.kind === 'parade' || pane.kind === 'histogram' ? pane[pane.kind].reference : null;
  if (reference?.shown) {
    const { width, height } = graticule;
    for (const [code, resting] of [[reference.low, REFERENCE_REST.low], [reference.high, REFERENCE_REST.high]] as const) {
      if (code === resting) continue;
      const signal = Math.min(1023, Math.max(0, code)) / 1023;
      // Where the engine puts a level: the middle of its row or column (`graticule.rs`, `levels`).
      if (pane.kind === 'histogram') {
        const x = (width - 1) * signal + 0.5;
        drawn.push({ x1: x, y1: 0, x2: x, y2: height, colour: REFERENCE, dashed: true });
      } else {
        const y = (height - 1) * (1 - signal) + 0.5;
        drawn.push({ x1: 0, y1: y, x2: width, y2: y, colour: REFERENCE, dashed: true });
      }
    }
  }
  return { lines: drawn, labels };
}

/** Where a picture of `width` × `height` stands in a pane of `paneWidth` × `paneHeight`: as large as fits, in the middle. */
export function fitted(width: number, height: number, paneWidth: number, paneHeight: number): { x: number; y: number; scale: number } {
  if (width <= 0 || height <= 0 || paneWidth <= 0 || paneHeight <= 0) return { x: 0, y: 0, scale: 1 };
  const scale = Math.min(paneWidth / width, paneHeight / height);
  return { x: (paneWidth - width * scale) / 2, y: (paneHeight - height * scale) / 2, scale };
}

// ---- The pictures ----------------------------------------------------------------------------

/** The bytes of a picture of `scopes.frame`, or null where its base64 is not `width × height × 4` bytes. */
export function pictureBytes(rgba: string, width: number, height: number): Uint8ClampedArray | null {
  let text: string;
  try { text = atob(rgba); } catch { return null; }
  if (text.length !== width * height * 4) return null;
  const bytes = new Uint8ClampedArray(text.length);
  for (let at = 0; at < text.length; at++) bytes[at] = text.charCodeAt(at);
  return bytes;
}

/**
 * Draw a picture of a scope into `canvas`, as large as the picture: its bytes put whole as the
 * image data of the canvas, which the page then scales to the pane. False where it could not be
 * drawn (its bytes are not those of its size, or the canvas has no 2D context).
 */
export function drawPicture(canvas: HTMLCanvasElement, rgba: string, width: number, height: number): boolean {
  const bytes = pictureBytes(rgba, width, height);
  if (bytes === null) return false;
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  let context: CanvasRenderingContext2D | null = null;
  try { context = canvas.getContext('2d'); } catch { /* A canvas that cannot draw. */ }
  if (!context) return false;
  const image = context.createImageData(width, height);
  image.data.set(bytes);
  context.putImageData(image, 0, 0);
  return true;
}

// ---- The preferences -----------------------------------------------------------------------

export const scopesStorageKey = 'columbina-scopes';

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const oneOf = <T extends string | number>(value: unknown, allowed: readonly T[], otherwise: T): T => (allowed.includes(value as T) ? value as T : otherwise);
const unit = (value: unknown, otherwise: number) => (typeof value === 'number' && Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : otherwise);
const flag = (value: unknown, otherwise: boolean) => (typeof value === 'boolean' ? value : otherwise);
const code = (value: unknown, otherwise: number) => (typeof value === 'number' && Number.isFinite(value) ? Math.round(Math.min(1023, Math.max(0, value))) : otherwise);

function readReference(value: unknown): ReferenceLevels {
  const given = isObject(value) ? value : {};
  return { shown: flag(given.shown, false), low: code(given.low, REFERENCE_REST.low), high: code(given.high, REFERENCE_REST.high) };
}

/** A pane as it was kept, each control that is missing or wrong as it is at first. */
export function readPane(value: unknown, kind: ScopeKind = 'parade'): PaneSettings {
  const given = isObject(value) ? value : {};
  const base = newPane(oneOf(given.kind, SCOPE_KINDS, kind));
  const of = (name: string) => (isObject(given[name]) ? given[name] as Record<string, unknown> : {});
  const waveform = of('waveform');
  const parade = of('parade');
  const vector = of('vectorscope');
  const histogram = of('histogram');
  const cie = of('chromaticity');
  return {
    kind: base.kind,
    waveform: {
      channels: oneOf(waveform.channels, ['luma', 'rgb'] as const, base.waveform.channels),
      brightness: unit(waveform.brightness, 0.5), graticule: unit(waveform.graticule, 0.5), reference: readReference(waveform.reference),
    },
    parade: { brightness: unit(parade.brightness, 0.5), graticule: unit(parade.graticule, 0.5), reference: readReference(parade.reference) },
    vectorscope: {
      tones: oneOf(vector.tones, ['all', 'low', 'mid', 'high'] as const, 'all'),
      low: unit(vector.low, 0.3), high: unit(vector.high, 0.7),
      zoom2x: flag(vector.zoom2x, false), skinTone: flag(vector.skinTone, false), showGraticule: flag(vector.showGraticule, true),
      brightness: unit(vector.brightness, 0.5), graticule: unit(vector.graticule, 0.5),
    },
    histogram: {
      channels: oneOf(histogram.channels, ['rgb', 'yrgb'] as const, 'rgb'),
      brightness: unit(histogram.brightness, 0.5), graticule: unit(histogram.graticule, 0.5), reference: readReference(histogram.reference),
    },
    chromaticity: {
      extra: (GAMUTS as readonly unknown[]).includes(cie.extra) ? cie.extra as Gamut : null,
      showGraticule: flag(cie.showGraticule, true), brightness: unit(cie.brightness, 0.5), graticule: unit(cie.graticule, 0.5),
    },
  };
}

/** The scopes as they were kept in `text`, each part that is missing or wrong as it is at first. */
export function readScopes(text: string | null): ScopesSettings {
  const base = defaultScopes();
  let given: unknown = null;
  try { given = text === null ? null : JSON.parse(text); } catch { given = null; }
  if (!isObject(given)) return base;
  const options = isObject(given.options) ? given.options : {};
  const floating = isObject(given.floating) ? given.floating : {};
  const panes = Array.isArray(floating.panes) ? floating.panes : [];
  const rect = isObject(floating.rect) && ['x', 'y', 'width', 'height'].every(each => typeof (floating.rect as Record<string, unknown>)[each] === 'number'
    && Number.isFinite((floating.rect as Record<string, number>)[each])) ? floating.rect as unknown as FloatRect : null;
  return {
    options: {
      scale: oneOf(options.scale, ['10-bit', 'percent', 'nits'] as const, base.options.scale),
      vectorStyle: oneOf(options.vectorStyle, ['standard', 'off'] as const, base.options.vectorStyle),
      targets: oneOf(options.targets, ['75', '100', 'both'] as const, base.options.targets),
      hueLabels: flag(options.hueLabels, base.options.hueLabels),
    },
    docked: readPane(given.docked, base.docked.kind),
    floating: {
      layout: oneOf(floating.layout, LAYOUTS, base.floating.layout),
      panes: base.floating.panes.map((pane, at) => readPane(panes[at], pane.kind)),
      rect: rect === null ? null : { x: rect.x, y: rect.y, width: Math.max(160, rect.width), height: Math.max(120, rect.height) },
    },
  };
}

/** The scopes as they were kept last on this desktop, or as they are at first. */
export function loadScopes(): ScopesSettings {
  try { return readScopes(localStorage.getItem(scopesStorageKey)); } catch { return defaultScopes(); }
}
export function storeScopes(settings: ScopesSettings): void {
  try { localStorage.setItem(scopesStorageKey, JSON.stringify(settings)); } catch { /* Without storage they hold for this window. */ }
}

/** How many scopes one connection may have at once (PROTOCOL.md, "Scopes"). */
export const MOST_SCOPES = 16;
