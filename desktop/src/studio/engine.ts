/**
 * The engine of the studio as the page sees it: the project as `project.get` gives it, the
 * operations that change it, what the engine tells, and the page's own copy of the project,
 * kept as the engine tells of every change. See engine/PROTOCOL.md.
 *
 * The page reaches the engine only through the shell (`window.columbina.engine`): it names no
 * path to it, and holds no handle of the window.
 */
import type { EngineBridge, EngineCallContext } from '../bridge';

/** A rate of frames, as a fraction: 24000/1001 is 23.976 frames a second. */
export interface Rate {
  num: number;
  den: number;
}

/** A place in a source: a count of ticks, each `1/scale` of a second. */
export interface Time {
  ticks: number;
  scale: number;
}

/** What tells a file from another one at the same path. */
export interface Identity {
  size: number;
  modified_us: number;
  digest: string;
}

/** The picture of a file. */
export interface VideoFacts {
  /** A decoded still holds its frame for the entire clip. */
  still?: boolean;
  width: number;
  height: number;
  rate: Rate;
  codec: string;
  bits: number;
  /** How far the picture is turned clockwise to be shown upright, in degrees. */
  rotation: number;
}

/** The sound of a file. */
export interface AudioFacts {
  rate: number;
  channels: number;
  codec: string;
}

/** What a file holds. A file without a picture has no `video`, one without sound no `audio`. */
export interface Facts {
  duration: Time;
  video?: VideoFacts | null;
  audio?: AudioFacts | null;
}

/** A copy of the picture of media whose every frame is a key frame, made by the engine. */
export interface Proxy {
  path: string;
  width: number;
  height: number;
  codec: string;
  digest_of_source: string;
}

/** A file that the project is cut from. `proxy` is left out where it has none. */
export interface Media {
  id: string;
  name: string;
  bin: string | null;
  /** The start of the first frame marked. */
  mark_in: Time | null;
  /**
   * Where the range marked ends: the start of the frame after the Out frame, which is in it, as
   * DaVinci Resolve's Out point is the last frame (Reference Manual 20, pp. 760, 824). Pressing O
   * at frame n keeps the start of n + 1; the Out is shown as frame n (see `outFrame`).
   */
  mark_out: Time | null;
  path: string;
  identity: Identity;
  facts: Facts;
  proxy?: Proxy;
  /**
   * Its input colour space and data levels set by hand. The engine always writes it; media that a
   * client builds may leave it out, and is read as none set (`keptOverride`).
   */
  color_override?: ColorOverride;
}

export type TrackKind = 'video' | 'audio' | 'subtitle';
export type Monitor = 'source' | 'timeline';
export interface Bin { id: string; name: string; parent: string | null }
export interface Speed { num: number; den: number }
export interface Transform { x: number; y: number; scale: number; rotation: number; opacity: number }
export interface TitleStyle { font: string; size: number; color: string; x: number; y: number; align: 'left' | 'center' | 'right' }
export type ClipSource =
  | { kind: 'media'; media: string; in: Time }
  | { kind: 'title'; text: string; style: TitleStyle }
  | { kind: 'color'; color: string };
export interface Clip {
  id: string;
  start: number;
  length: number;
  source: ClipSource;
  enabled: boolean;
  speed: Speed;
  reverse: boolean;
  link: string | null;
  transform: Transform;
  gain_db: number;
  pan: number;
  fade_in: number;
  fade_out: number;
  /** Its grade, which colours its picture before it is put on the tracks below it. A clip of sound keeps one too. */
  grade: Grade;
  /** The group that it is graded in, or null. */
  group: string | null;
}
export type TransitionKind = 'dissolve' | 'dip_black' | 'dip_white' | 'crossfade';
export type Alignment = 'start' | 'center' | 'end';
export interface Transition { id: string; kind: TransitionKind; left: string; right: string; length: number; align: Alignment }
export interface Cue { id: string; start: number; length: number; text: string }
interface TrackBase { id: string; name: string; enabled: boolean; locked: boolean }
export interface ClipTrack extends TrackBase { kind: 'video' | 'audio'; clips: Clip[]; transitions: Transition[] }
export interface SubtitleTrack extends TrackBase { kind: 'subtitle'; language: string; cues: Cue[] }
export type Track = ClipTrack | SubtitleTrack;
export interface Timeline {
  id: string;
  name: string;
  rate: Rate;
  width: number;
  height: number;
  sample_rate: number;
  tracks: Track[];
  /** Its grade, which colours the picture once its tracks are put together. */
  grade: Grade;
}
/**
 * A project. `color` is always written by the engine; a project that a client builds may leave it
 * out, and it is read as not colour managed (`keptColor`).
 */
export interface Project { schema: 3; next_id: number; bins: Bin[]; media: Media[]; timelines: Timeline[]; groups: Group[]; color?: ColorSettings }

// Colour management (PROTOCOL.md, "Colour management"; engine/crates/model/src/color_management.rs),
// as DaVinci Resolve's Project Settings > Color Management sets it (Reference Manual 20, pp. 149-153,
// 224-252). Spaces are named as the engine's catalogue names them.

/** The Color science menu (pp. 149, 224-226). */
export type ColorScience = 'yrgb' | 'yrgb_managed' | 'acescct' | 'acescc';
/** The Resolve color management preset (pp. 230-232). */
export type ColorPreset = 'sdr_rec709' | 'sdr_p3_broadcast' | 'sdr_p3_cinema' | 'sdr_rec2020' | 'davinci_wide_gamut' | 'hdr_p3_broadcast' | 'hdr_rec2020' | 'custom';
/** Timeline working luminance (p. 238); `custom` takes `custom_luminance`. */
export type WorkingLuminance = 'sdr_100' | 'hdr_500' | 'hdr_1000' | 'hdr_2000' | 'hdr_4000' | 'hdr_er_1000_2000' | 'custom';
/** A DRT of input or output (pp. 239-242). Only `none` is a plain transform; the others are the engine's own. */
export type ToneMapping = 'none' | 'simple' | 'luminance' | 'davinci' | 'saturation_preserving';
/** A gamut, as the model names it (Limit output gamut to, p. 239). */
export type ColorGamut = 'rec709' | 'rec601-pal' | 'rec601-ntsc' | 'rec2020' | 'dci-p3' | 'p3-d65';
/** Apply resize transformations in (p. 243). */
export type ResizeSpace = 'timeline' | 'log' | 'linear' | 'linear_mapped' | 'gamma' | 'gamma_mapped';
/** The 1D and 3D LUT of a stage (pp. 151-152), by names relative to the folders of LUTs; the 1D is done first. */
export interface LutPair { '1d': string | null; '3d': string | null }
/** Sat. Rolloff Start and Limit of a Saturation Preserving DRT, in nits (p. 240). */
export interface SatRolloff { start: number; limit: number }
/** The colour management of a project. Every field is always written by the engine. */
export interface ColorSettings {
  science: ColorScience;
  automatic: boolean;
  processing_mode: 'sdr' | 'hdr';
  preset: ColorPreset;
  /** A space of the catalogue, or `bypass`. */
  output: string;
  separate_gamma: boolean;
  /** A space of the catalogue, or `bypass`. */
  input: string;
  timeline: string;
  working_luminance: WorkingLuminance;
  custom_luminance: number;
  hdr_203_nits: boolean;
  limit_gamut: ColorGamut | null;
  input_drt: ToneMapping;
  input_sat_rolloff: SatRolloff;
  output_drt: ToneMapping;
  output_sat_rolloff: SatRolloff;
  inverse_drt: boolean;
  white_point_adaptation: boolean;
  color_space_aware_tools: boolean;
  resize_in: ResizeSpace;
  graphics_white: number;
  aces_version: '1.3' | '2.0';
  /** A space of the catalogue. */
  idt: string;
  /** An ODT of `aces_version`, by its name. */
  odt: string;
  gamut_compress: boolean;
  node_luts_in: 'timeline' | 'ap0_linear';
  lut_interpolation: 'trilinear' | 'tetrahedral';
  input_lut: LutPair;
  output_lut: LutPair;
  viewer_lut: LutPair;
  s_curve_contrast: boolean;
}
/** Data Levels of media (pp. 221-222). */
export type DataLevels = 'auto' | 'video' | 'full';
/**
 * The colour of media set by hand: a space of the catalogue, `bypass`, or null for the project's default; its data levels; the
 * LUTs of the source clip (pp. 3409-3410); and whether its Inverse DRT for SDR to HDR Conversion is on (p. 233).
 */
export interface ColorOverride { input: string | null; levels: DataLevels; lut: LutPair; inverse_drt: boolean }

/** The colour settings of a new project, as the engine writes them: not managed (DaVinci YRGB, p. 225). */
export function defaultColorSettings(): ColorSettings {
  const rec709 = 'Rec.709 Gamma 2.4';
  const noLuts = (): LutPair => ({ '1d': null, '3d': null });
  return {
    science: 'yrgb', automatic: false, processing_mode: 'sdr', preset: 'sdr_rec709', output: rec709,
    separate_gamma: false, input: rec709, timeline: rec709, working_luminance: 'sdr_100', custom_luminance: 100,
    hdr_203_nits: false, limit_gamut: null, input_drt: 'none', input_sat_rolloff: { start: 100, limit: 1000 },
    output_drt: 'none', output_sat_rolloff: { start: 100, limit: 1000 }, inverse_drt: false,
    white_point_adaptation: true, color_space_aware_tools: true, resize_in: 'linear', graphics_white: 100,
    aces_version: '2.0', idt: rec709, odt: 'Rec.709', gamut_compress: false, node_luts_in: 'timeline',
    lut_interpolation: 'trilinear', input_lut: noLuts(), output_lut: noLuts(), viewer_lut: noLuts(), s_curve_contrast: true,
  };
}

/** The colour settings of a project as the engine reads them: each one left out is its default. */
export function keptColor(project: Pick<Project, 'color'>): ColorSettings {
  return mergePatch(defaultColorSettings(), project.color ?? {}) as ColorSettings;
}

/** The colour of media as the engine reads it: what is left out is none set. */
export function keptOverride(media: Pick<Media, 'color_override'>): ColorOverride {
  return { input: null, levels: 'auto', lut: { '1d': null, '3d': null }, inverse_drt: true, ...media.color_override };
}

// Grades (PROTOCOL.md, "Grades"; engine/crates/model/src/grade.rs), as DaVinci Resolve's Color
// page builds them (Reference Manual 20, Chapters 143–147).

/** How the value of a parameter goes from a key to the next: on a line, held, or along a curve. */
export type Interp = 'linear' | 'hold' | { bezier: { out: [number, number]; in: [number, number] } };
/**
 * A keyframe. Of a clip's grade (and its group's, as they apply to it): a frame of the clip's media
 * at the media's own rate, the number of the picture shown (of a title, a colour or a still, a frame
 * of the clip from 0); of a timeline's grade, a frame of the timeline. See the "Frames of keys" of
 * engine/PROTOCOL.md.
 */
export interface Keyframe { frame: number; value: number; interp: Interp }
/** A setting of a node: one value, or keyframes in the order of their frames, one at least. */
export type Param = number | { keys: Keyframe[] };
export interface Wheel { y: Param; r: Param; g: Param; b: Param }
export interface Rgb { r: Param; g: Param; b: Param }
export interface Primaries {
  lift: Wheel; gamma: Wheel; gain: Wheel; offset: Rgb;
  contrast: Param; pivot: Param; saturation: Param; hue: Param; lum_mix: Param; temperature: Param; tint: Param;
  color_boost: Param; midtone_detail: Param; shadows: Param; highlights: Param;
}
export interface LogWheels { shadow: Rgb; midtone: Rgb; highlight: Rgb; low_range: Param; high_range: Param }
/** A curve point: `[in, out]`. */
export type CurvePoint = [number, number];
export interface Curve { points: CurvePoint[]; intensity: Param }
export interface Curves { ganged: boolean; y: Curve; r: Curve; g: Curve; b: Curve }
export interface HslCurves {
  hue_vs_hue: CurvePoint[]; hue_vs_sat: CurvePoint[]; hue_vs_lum: CurvePoint[];
  lum_vs_sat: CurvePoint[]; sat_vs_sat: CurvePoint[]; sat_vs_lum: CurvePoint[];
}
export interface SoftClip { low: Rgb; high: Rgb; low_soft: Rgb; high_soft: Rgb }
export interface KeyInput { invert: boolean; matte: boolean; gain: Param; offset: Param }
export interface KeyOutput { invert: boolean; gain: Param; offset: Param }
/**
 * The adjustment Shot Match puts last in a node, which no palette shows (p. 3048): each channel
 * `x` goes to `S(gain * x^gamma + offset)`, `S` an S-curve of `contrast` about `pivot`. Neutral:
 * gamma 1, gain 1, offset 0, contrast 1 (pivot 0.435). See PROTOCOL.md, "Grades".
 */
export interface ShotMatch { gamma: Rgb; gain: Rgb; offset: Rgb; contrast: Param; pivot: Param }
/** The blur of the key coming into a corrector (Key palette, p. 3362): radius 0…2000 (0), ratio 0…1 (0.5). */
export interface KeyBlur { radius: Param; ratio: Param }
/** The tab of the Blur palette (pp. 3454–3456). */
export type BlurMode = 'blur' | 'sharpen' | 'mist';
/**
 * The Blur palette of a corrector (pp. 3454–3456): one set of settings that its tabs show. Radius,
 * H/V Ratio (`ratio`) and Scaling are R, G and B (0.5, 0.5, 0.25 at first, 0…1), ganged by `linked`;
 * Coring Softness and Level 0…1 (0), Mix 0…100 (100). See PROTOCOL.md, "Grades".
 */
export interface BlurSettings {
  mode: BlurMode; radius: Rgb; ratio: Rgb; scaling: Rgb; coring_softness: Param; level: Param; mix: Param;
  linked: { radius: boolean; ratio: boolean; scaling: boolean };
}
/**
 * The Key palette of a corrector (pp. 3361–3362). `qualifier` (the Qualifier controls: invert, Matte
 * mode, gain, offset; at first `false`, `true`, 1, 0) and `input_blur` are always written by the
 * engine, and read as they are at first where they are left out, as they are here.
 */
export interface CorrectorKey { input: KeyInput; output: KeyOutput; qualifier?: KeyInput; input_blur?: KeyBlur }
/** The modes of the Qualifier palette that are kept (pp. 3160–3175); HSL by default. */
export type QualifierMode = 'hsl' | 'rgb' | 'lum';
/** Hue of an HSL qualifier, each 0…100: a turn of hue (red 0, green 33.3, blue 66.7), going round. */
export interface HueRange { on: boolean; center: Param; width: Param; soft: Param; symmetry: Param }
/** A range of saturation, luma, red, green or blue of a qualifier, each 0…100, black to white. */
export interface QualifierRange { on: boolean; low: Param; high: Param; low_soft: Param; high_soft: Param }
/** Matte Finesse's Morph Operation (p. 3181). */
export type Morph = 'shrink' | 'grow' | 'opening' | 'closing';
/** The Matte Finesse controls that are kept (pp. 3178–3181); each changes nothing at its default. */
export interface MatteFinesse {
  pre_filter: Param; clean_black: Param; black_clip: Param; clean_white: Param; white_clip: Param;
  blur_radius: Param; in_out_ratio: Param; morph: Morph; morph_radius: Param;
}
/**
 * The Qualifier palette of a corrector (pp. 3157–3182): its own key, of the picture that comes into it.
 * Given whole to a node that has none (`set_node {qualifier: {...}}`), patched in part, taken away
 * with `{qualifier: null}`. Defaults, ranges and the order of the key: PROTOCOL.md, "Grades".
 */
export interface Qualifier {
  mode: QualifierMode; invert: boolean;
  hue: HueRange; sat: QualifierRange; lum: QualifierRange; red: QualifierRange; green: QualifierRange; blue: QualifierRange;
  finesse: MatteFinesse;
}
/** Power Window points are centred on the window: x right, y down, one unit one image height. */
export interface WindowPoint { x: Param; y: Param }
/** The handles are offsets from the anchor, in the same local space. */
export interface WindowBezierPoint { position: WindowPoint; in: WindowPoint; out: WindowPoint }
/**
 * Transform controls (RM20 pp. 3185–3199): Size/Aspect/Pan/Tilt 50, Rotate 0.
 * Pan goes right, Tilt goes up, Rotate clockwise. Numeric mapping/ranges are project choices
 * documented in PROTOCOL.md; the model supplies the shared forward and inverse transform.
 */
export interface WindowTransform { size: Param; aspect: Param; pan: Param; tilt: Param; rotate: Param }
export type WindowShape =
  | { kind: 'linear'; points: [WindowPoint, WindowPoint, WindowPoint, WindowPoint]; softness: [Param, Param, Param, Param] }
  | { kind: 'circular'; softness: Param }
  | { kind: 'polygon'; points: WindowPoint[]; inside: Param; outside: Param }
  | { kind: 'curve'; points: WindowBezierPoint[]; closed: boolean; inside: Param; outside: Param }
  | { kind: 'gradient'; softness: Param };
/** A window id is local to its node. Presets copy the complete list into the project. */
export interface PowerWindow {
  id: string; label: string; enabled: boolean; invert: boolean; combine: 'add' | 'subtract';
  transform: WindowTransform; opacity: Param; shape: WindowShape;
  /** T24: its tracking, apart from its own transform; left out when there is none. */
  tracking?: WindowTracking;
}
/** One of the eight curves of the tracker graph (PROTOCOL.md, "Window tracking"). */
export type TrackComponent = 'pan' | 'tilt' | 'zoom' | 'rotate' | 'aspect' | 'shear' | 'yaw' | 'pitch';
export type TrackMotion = Record<TrackComponent, number>;
/** The five boxes of the Tracker palette (p. 3235). */
export interface TrackComponents { pan: boolean; tilt: boolean; zoom: boolean; rotate: boolean; perspective: boolean }
/** Frames are the grade's clock, written as JSON keys. */
export interface TrackData {
  anchor: [number, number]; frame_size: [number, number]; components: TrackComponents;
  curves: Partial<Record<TrackComponent, Record<string, number>>>;
  confidence: Record<string, number>;
  points: Record<string, [number, number]>[];
  keys: Record<string, TrackMotion>;
}
export interface WindowTracking { mode?: 'clip' | 'frame'; crosshairs?: [number, number][]; data: TrackData }
export type TrackDirection = 'forward' | 'reverse' | 'forward_and_back' | 'one_forward' | 'one_reverse';
export type TrackEdit =
  | { kind: 'clear' | 'interpolate'; from: number; to: number; components?: TrackComponent[] }
  | { kind: 'reset' } | { kind: 'mode'; mode: 'clip' | 'frame' }
  | { kind: 'add_key' | 'delete_key'; frame: number }
  | { kind: 'crosshairs'; points: [number, number][] }
  | { kind: 'paste'; from: string };
export interface TrackStarted { job: string; first: number; last: number; frame_size: [number, number]; direction: TrackDirection }
/** `track.progress`, to the connection that started the track. */
export interface TrackProgress { job: string; frame: number; timeline_frame: number | null; done: number; total: number; lost: number; revision: number }
/** `track.state`: how a track ended. */
export interface TrackEnded {
  job: string; phase: 'done' | 'stopped' | 'failed'; reason: string | null; frames: number; lost: number; frame: number;
  revision: number; tracked: [number, number] | null; message?: string;
}
export interface Corrector {
  primaries: Primaries; log: LogWheels; curves: Curves; hsl: HslCurves; lut: string | null; soft_clip: SoftClip;
  key: CorrectorKey;
  /** The engine always writes it; `null` for none. */
  shot_match: ShotMatch | null;
  /** The engine always writes it; older files leave it out, which is the palette as at first. */
  blur?: BlurSettings;
  /** The engine always writes it; `null` for none, as a node is until a colour is picked. */
  qualifier?: Qualifier | null;
  /** The engine writes the whole list; omitted by older files, which have no windows. */
  windows?: PowerWindow[];
}

/** A qualifier as the engine makes one first: every pixel, Matte Finesse at its defaults. */
export function newQualifier(): Qualifier {
  const range = (): QualifierRange => ({ on: true, low: 0, high: 100, low_soft: 0, high_soft: 0 });
  return {
    mode: 'hsl', invert: false,
    hue: { on: true, center: 50, width: 100, soft: 0, symmetry: 50 },
    sat: range(), lum: range(), red: range(), green: range(), blue: range(),
    finesse: {
      pre_filter: 0, clean_black: 0, black_clip: 0, clean_white: 0, white_clip: 100,
      blur_radius: 0, in_out_ratio: 0, morph: 'shrink', morph_radius: 0,
    },
  };
}

/** What `viewer.highlight` asks for and answers (PROTOCOL.md, "Highlight"): Shift-H `highlight`, Alt-Shift-H `bw`. */
export type HighlightMode = 'off' | 'highlight' | 'bw';
export interface ViewerHighlight { monitor: 'timeline'; mode: HighlightMode; grade?: Target; node?: string }
export type Composite =
  | 'normal' | 'add' | 'color' | 'color_burn' | 'color_dodge' | 'darken' | 'darker_color' | 'difference' | 'divide'
  | 'exclusion' | 'hard_mix' | 'hardlight' | 'hue' | 'lighten' | 'lighter_color' | 'linear_burn' | 'linear_dodge'
  | 'linear_light' | 'luminosity' | 'multiply' | 'overlay' | 'pin_light' | 'saturation' | 'screen' | 'softlight'
  | 'subtract' | 'vivid_light';
export interface ParallelMixer { inputs: number }
export interface LayerMixer { inputs: number; composite: Composite }
export interface KeyMixer { inputs: KeyInput[]; output: KeyOutput }
/** Where a node lies in the node editor: the top left corner of its card, a pixel at zoom 1 (PROTOCOL.md, "A grade"). */
export interface NodePos { x: number; y: number }
/**
 * A node of a grade (named so as not to hide the DOM's `Node`). Its id is unique within its grade
 * only, and never `source` or `output`; its `params` are those of its kind. The engine always
 * writes `locked` (Lock Node, p. 3382) and `pos` (`null` where the editor lays it out), and reads
 * them as `false` and `null` where they are left out, as they are here.
 */
export type GradeNode = { id: string; label: string; enabled: boolean; locked?: boolean; pos?: NodePos | null } & (
  | { kind: 'corrector'; params: Corrector }
  | { kind: 'parallel_mixer'; params: ParallelMixer }
  | { kind: 'layer_mixer'; params: LayerMixer }
  | { kind: 'key_mixer'; params: KeyMixer });
export type NodeKind = GradeNode['kind'];
/** What a link carries: the picture or a key. */
export type LinkKind = 'rgb' | 'key';
/** A link from the output of `kind` of `from` (a node or `source`) into input `input` of `to` (a node or `output`). */
export interface Link { from: string; to: string; kind: LinkKind; input: number }
/** Nodes in the order they were added (an editor's numbering), links in the engine's order (see `sortLinks`). */
export interface Grade { nodes: GradeNode[]; links: Link[] }
/** One of the four grades a picture goes through, in this order: pre_clip, clip, post_clip, timeline. */
export type Target =
  | { level: 'clip'; timeline: string; clip: string }
  | { level: 'pre_clip'; group: string }
  | { level: 'post_clip'; group: string }
  | { level: 'timeline'; timeline: string };
/** A group of clips graded together; a clip names it in its `group`. */
export interface Group { id: string; name: string; pre: Grade; post: Grade }

/** The grade every new clip and timeline has, as the engine writes it: the source linked straight to the output. */
export function emptyGrade(): Grade {
  return { nodes: [], links: [{ from: 'source', to: 'output', kind: 'rgb', input: 0 }] };
}

/**
 * Rust compares the ids of links as strings, byte by byte in UTF-8, which is the order of code
 * points. JavaScript's `<` compares UTF-16 code units, which differs from it beyond U+FFFF, and ids
 * that a client gives are not held to ASCII; so code points are compared here.
 */
function compareIds(a: string, b: string): number {
  const shorter = Math.min(a.length, b.length);
  for (let at = 0; at < shorter;) {
    const x = a.codePointAt(at) as number;
    const y = b.codePointAt(at) as number;
    if (x !== y) return x < y ? -1 : 1;
    at += x > 0xffff ? 2 : 1;
  }
  return a.length - b.length;
}

const KIND_ORDER: Record<LinkKind, number> = { rgb: 0, key: 1 };

/** The links of a grade in the engine's one order: by `to`, then kind (rgb before key), then input. */
export function sortLinks(links: readonly Link[]): Link[] {
  return [...links].sort((a, b) => compareIds(a.to, b.to) || KIND_ORDER[a.kind] - KIND_ORDER[b.kind] || a.input - b.input);
}

type Settings<T, K extends keyof T> = { [P in K]?: T[P] | null };
/** Basic operations broadcast in `done`. Editing commands are expanded by the engine alone. */
export type Op =
  | { op: 'add_media'; media: Media; index?: number | null }
  | { op: 'remove_media'; id: string }
  | ({ op: 'set_media'; id: string; still?: boolean | null } & Settings<Media, 'name' | 'bin' | 'mark_in' | 'mark_out'>)
  | { op: 'set_proxy'; media: string; proxy: Proxy | null }
  | { op: 'add_bin'; bin: Bin; index?: number | null }
  | { op: 'remove_bin'; id: string }
  | ({ op: 'set_bin'; id: string } & Settings<Bin, 'name' | 'parent'>)
  | { op: 'add_timeline'; timeline: Timeline; index?: number | null }
  | { op: 'remove_timeline'; id: string }
  | ({ op: 'set_timeline'; id: string } & Settings<Timeline, 'name' | 'rate' | 'width' | 'height' | 'sample_rate'>)
  | { op: 'add_track'; timeline: string; track: Track; index?: number | null }
  | { op: 'remove_track'; timeline: string; id: string }
  | ({ op: 'set_track'; timeline: string; id: string; language?: string | null } & Settings<Track, 'name' | 'enabled' | 'locked'>)
  | { op: 'insert_clip'; timeline: string; track: string; clip: Clip }
  | { op: 'remove_clip'; timeline: string; id: string }
  | { op: 'move_clip'; timeline: string; id: string; track: string; start: number }
  | { op: 'shift_clips'; timeline: string; track: string; clips: string[]; by: number }
  | { op: 'shift_cues'; timeline: string; track: string; cues: string[]; by: number }
  | { op: 'trim_clip'; timeline: string; id: string; source_in?: Time | null; start: number; length: number }
  | ({ op: 'set_clip'; timeline: string; id: string } & Settings<Clip, 'enabled' | 'speed' | 'reverse' | 'link' | 'transform' | 'gain_db' | 'pan' | 'fade_in' | 'fade_out' | 'source'>)
  | { op: 'add_transition'; timeline: string; track: string; transition: Transition }
  | { op: 'remove_transition'; timeline: string; id: string }
  | ({ op: 'set_transition'; timeline: string; id: string } & Settings<Transition, 'kind' | 'length' | 'align'>)
  | { op: 'add_cue'; timeline: string; track: string; cue: Cue }
  | { op: 'remove_cue'; timeline: string; id: string }
  | ({ op: 'set_cue'; timeline: string; id: string } & Settings<Cue, 'start' | 'length' | 'text'>)
  /** In `done`: the id the engine gave, the index, and the links naming that id. */
  | { op: 'add_node'; grade: Target; node: GradeNode; index?: number | null; links?: Link[] }
  /** In `done`: always with the `links` put in the node's place. */
  | { op: 'remove_node'; grade: Target; id: string; links?: Link[] | null }
  | { op: 'connect'; grade: Target; link: Link }
  | { op: 'disconnect'; grade: Target; to: string; kind: LinkKind; input: number }
  /** `params` is a patch: each object is merged into the object of that name, any other value (null too) takes its place. */
  | { op: 'set_node'; grade: Target; id: string; label?: string | null; enabled?: boolean | null; locked?: boolean | null; pos?: NodePos | null; params?: Record<string, unknown> | null }
  | { op: 'set_grade'; grade: Target; value: Grade }
  | { op: 'add_group'; group: Group; index?: number | null }
  | { op: 'remove_group'; id: string }
  | { op: 'set_group'; id: string; name?: string | null }
  | { op: 'set_clip_group'; timeline: string; clip: string; group: string | null }
  /** `settings` is a patch of `color`, merged as `set_node` merges `params`; `done` and the inverse carry the values as the engine writes them. */
  | { op: 'set_color_settings'; settings: Record<string, unknown> }
  /** `input` left out keeps it; null sets it back to the project's default. */
  | { op: 'set_media_color'; id: string; input?: string | null; levels?: DataLevels | null; lut?: LutPair; inverse_drt?: boolean };

/** Who asked for a change, or made the step that it takes back or does again. */
export type ClientKind = 'desktop' | 'agent' | 'cli' | 'engine';

/** The notification `project.changed`. */
export interface ProjectChanged {
  revision: number;
  /**
   * The revision that `done` applies to, where it is not the one before `revision`: the changes of
   * a gesture told together (PROTOCOL.md, "Gestures").
   */
  base?: number;
  by: ClientKind;
  made_by: ClientKind;
  done: Op[];
  next_id: number;
}

/** The state of the transport: the result of `transport.load` and `transport.state`, and the notification `transport.state`. */
interface TransportBase {
  frame: number | null;
  playing: boolean;
  speed: number;
  exact: boolean;
  shown: number | null;
  last: number | null;
  sound: 'none' | 'on' | 'off' | 'unavailable';
  proxies: boolean;
}
export interface SourceTransportState extends TransportBase {
  monitor: 'source';
  media: string | null;
  source: 'proxy' | 'original' | null;
}
export interface TimelineTransportState extends TransportBase {
  monitor: 'timeline';
  timeline: string | null;
  source: 'proxy' | 'original' | 'mixed' | null;
  late: number;
  /** The Highlight mode shown (`viewer.highlight`), while one is on. */
  highlight?: { mode: 'highlight' | 'bw'; grade: Target; node: string };
  /** The LUTs that the grades of the timeline name and that cannot be read; left out where there are none (PROTOCOL.md, "Grades in the picture"). */
  missing_luts?: MissingLut[];
  /** Whether the picture is shown without its grades (`viewer.bypass`, Bypass Color), while it is. */
  bypass?: boolean;
  /** The LUT shown on a node in place of its own (`viewer.preview`, Live Preview), while one is. */
  preview?: LivePreview;
  /** False while the view leaves the Color Viewer LUT out (`viewer.lut`), off the Color page. */
  viewer_lut?: boolean;
  /** True while a change of the project's colour management is being made, the colour before it still shown. */
  color_pending?: boolean;
  /** Why the colour management that the project asks for could not be made, while it cannot. */
  color_error?: string;
  /** The frame of the timeline that each reference of the wipe or split screen shows for the frame shown, in their order; while one is on. */
  compared?: (number | null)[];
}
export type TransportState = SourceTransportState | TimelineTransportState;
/** A LUT that a grade of the timeline names and the engine could not read (PROTOCOL.md, "Grades in the picture"). */
export interface MissingLut { name: string; message: string }
/** A LUT of the library shown on a node of a grade in place of its own, at a frame where Hover Scrub asks for one (PROTOCOL.md, "Live Preview and the Color Viewer LUT"). */
export interface LivePreview { grade: Target; node: string; lut: string; frame?: number }
/** A preview of a LUT (`luts.preview`): its picture, or why it cannot be read. */
export type LutPreview = { lut: string; width: number; height: number; png: string } | { lut: string; error: string };

/** A folder that a corrector's `lut` is looked for in, in the order of `luts.list` (PROTOCOL.md, "The library of LUTs"). */
export interface LutRoot { place: 'project' | 'library'; path: string; exists: boolean }
/**
 * A LUT of the library, by the name that a node gives it (its path from its root, with `/`); `root`
 * is the index of its root, and `shadowed` says that a root before it has one of the same name, which is read instead.
 */
export interface LutFile {
  name: string; root: number; size: number; modified_us: number | null; shadowed: boolean;
  /** Its tables (null where it cannot be told; left out by an engine from before it said). */
  kind?: LutKind | null;
  /** Whether it parses as the engine grades with it: `error` cannot be used (`message` says why); `unchecked` was not parsed by the list. */
  status?: 'ok' | 'error' | 'unchecked';
  message?: string;
}
/** What tables a `.cube` has: a 1D table, a cube, or a 1D shaper before a cube (Resolve's). */
export type LutKind = '1d' | '3d' | 'shaper+3d';
/** The answer of `luts.list`. */
export interface LutLibrary { roots: LutRoot[]; luts: LutFile[]; truncated: boolean }
/** What a view shows, as `view.display` tells it: whether Windows has HDR on for its monitor, and whether it shows HDR. */
export interface ViewShows { hdr_on: boolean; hdr_shown: boolean; peak_nits: number | null; sdr_white_nits: number | null }
/** The answer of `view.display` (Display HDR On Viewers If Available, p. 244), for each monitor that has a view. */
export interface ViewDisplay { hdr: boolean; source: ViewShows | null; timeline: ViewShows | null }

/** Viewer-only geometry. Scale is device pixels per oriented picture pixel; center may lie outside 0..1. */
export type Viewport = { mode: 'fit' } | { mode: 'scale'; scale: number; center: { x: number; y: number } };
export type ViewViewport = { monitor: Monitor } & Viewport;
/** Omit mode to query the requested setting; native presentation may still be pending. */
export type ViewportParams = { monitor: Monitor } | ViewViewport | ViewNavigation | { monitor: Monitor; operation: 'prepare'; instance?: number };
export interface ViewGeometry { surface: [number, number]; picture: [number, number] }
export type ViewNavigation = { monitor: Monitor; instance: number } & (
  { operation: 'zoom'; factor: number; at?: { x: number; y: number } }
  | { operation: 'pan'; base: Viewport; geometry: ViewGeometry; offset: { x: number; y: number } }
  | { operation: 'scale'; scale: number } | { operation: 'fit' });
export type PreparedViewport = ViewViewport & { instance: number; geometry: ViewGeometry; buttons?: ViewPointer['buttons'] };

/** The notification `media.proxy`: how the proxy of media is being made. */
export interface ProxyEvent {
  media: string;
  state: 'queued' | 'running' | 'done' | 'failed';
  progress?: number;
  message?: string;
}

/** The notification `engine.error`. */
export interface EngineErrorNotice {
  message: string;
  kind: string;
}

/** The result of `project.get`. */
export interface ProjectGot {
  revision: number;
  next_id: number;
  project: Project;
}

/** The result of `transaction.apply`: the operations as they were done, and the ids that `$names` were given. */
export interface Applied {
  revision: number;
  done: Op[];
  next_id: number;
  names: Record<string, string>;
  /** Input edit indexes whose requested amount exceeded the available handles. */
  clamped: number[];
}

/** The result of `history.undo` and `history.redo`: `done` is null where there was nothing to take back or do again. */
export interface Stepped {
  revision: number;
  done: Op[] | null;
  next_id?: number;
}

/** The result of `media.import`: the media added, in the order of the files. */
export interface Imported {
  revision: number;
  added: string[];
  next_id: number;
}

/** Export options from the page. Only the native Save As dialog supplies the output path. */
export interface RenderOptions {
  timeline: string;
  codec: 'h264' | 'hevc' | 'prores';
  quality: 'high' | 'medium' | 'small';
  range?: { from: number; to: number };
  /**
   * The subtitle tracks of the timeline whose SRT is saved beside the film, shown or hidden, as the
   * checkboxes of DaVinci Resolve's Deliver page choose them (Reference Manual 20, p. 1102).
   */
  subtitle_tracks: string[];
  language?: string;
  /** The name of the timeline, which Save As suggests to name the file after. */
  name?: string;
}
export type RenderStarted = { job: string } | { cancelled: true };

/** What the engine tells of an export as it goes, at most every 250 ms, and once at its end (`render.progress`). */
export interface RenderProgress {
  job: string;
  state: 'running' | 'done' | 'failed' | 'cancelled';
  /** Frames done, and of how many. */
  frame: number;
  frames: number;
  fps: number;
  /** Where the film is written: another file than the one chosen where ProRes was written instead. */
  path?: string;
  /** The encoder of libavcodec, once one is open, such as `h264_nvenc` or `prores_ks`. */
  encoder?: string;
  /** Why it failed, that it fell back to ProRes, or that the graphics device was made again. */
  message?: string;
}

/**
 * What became of one old project of the folder as the studio was opened: the timeline that it
 * became and what of it was left out, or why it could not be migrated (`error`).
 */
export interface MigrationReport {
  /** The folder of the old project. */
  source: string;
  timeline?: string;
  dropped?: string[];
  warnings?: string[];
  error?: string;
}

/** The methods of the engine that the page may call, and what each answers. */
export interface PageMethods {
  'project.get': ProjectGot;
  'transaction.apply': Applied;
  'history.undo': Stepped;
  'history.redo': Stepped;
  'media.make_proxy': Record<string, never>;
  'timeline.duration': { frames: number };
  'media.thumbnail': { width: number; height: number; png: string };
  'media.waveform': { rate: number; channels: number; peaks: string } | { pending: true };
  'luts.list': LutLibrary;
  'view.display': ViewDisplay;
  'view.viewport': ViewViewport & Partial<{ instance: number; geometry: ViewGeometry; buttons: ViewPointer['buttons'] }>;
  'render.cancel': Record<string, never>;
  'transport.load': TransportState;
  'transport.play': Record<string, never>;
  'transport.pause': Record<string, never>;
  'transport.seek': Record<string, never>;
  'transport.step': Record<string, never>;
  'transport.state': TransportState;
  'transport.proxies': Record<string, never>;
  'view.place': Record<string, never>;
  'view.show': Record<string, never>;
  'scopes.statistics': ScopeStatistics;
  'scopes.subscribe': ScopeAnswer;
  'scopes.set': ScopeAnswer;
  'scopes.unsubscribe': Record<string, never>;
  'view.sample': ViewSample;
  'view.pointer': { monitor: Monitor; on: boolean; cursor: PointerCursor | null };
  'window.bind': { binding: number; sequence: number };
  'window.input': WindowState;
  'window.unbind': { binding: number; phase: 'ended' };
  'track.start': TrackStarted;
  'track.stop': Record<string, never>;
  'track.edit': { revision: number };
  // The Gallery and the comparisons of the viewer (PROTOCOL.md, "The Gallery", "Comparisons in the viewer").
  'viewer.wipe': Compared;
  'viewer.split': Compared;
  'still.grab': Still;
  'still.get': StillWhole;
  'still.apply': Applied;
  'still.label': { id: string; label: string };
  'still.delete': { deleted: string[] };
  'still.restore': { restored: Record<string, string> };
  'still.move': { moved: Record<string, string> };
  'still.copy': { copied: Record<string, string> };
  'gallery.list': { albums: GalleryAlbum[]; memories?: GalleryAlbum | null };
  'gallery.thumbnail': { width: number; height: number; png: string };
  'gallery.album.create': { id: string; name: string; kind: AlbumKind };
  'gallery.album.rename': { id: string; name: string };
  'gallery.album.delete': { deleted: string[] };
  'grade.append_node': Applied;
  'viewer.highlight': ViewerHighlight;
  'viewer.bypass': { monitor: 'timeline'; on: boolean };
  'timeline.thumbnail': { width: number; height: number; png: string; frame: number };
  // Window presets, this user's and shared by every project (PROTOCOL.md, "Window presets").
  'window_presets.list': { presets: WindowPreset[] };
  'window_presets.save': WindowPreset;
  'window_presets.update': WindowPreset;
  'window_presets.delete': { deleted: string };
  'window_presets.apply': Applied;
  // The LUT browser's thumbnails and Live Preview, and the Color Viewer LUT on the Color page alone (PROTOCOL.md, "Previews of LUTs").
  'luts.preview': { previews: LutPreview[] };
  'viewer.preview': { monitor: 'timeline'; preview: LivePreview | null };
  'viewer.lut': { monitor: 'timeline'; on: boolean };
}
export type PageMethod = keyof PageMethods;

/**
 * A window preset (RM20 pp. 3198–3199): the windows of a node as they were on the frame of the
 * playhead, every parameter fixed, kept in this user's library apart from every project.
 */
export interface WindowPreset { id: string; name: string; windows: PowerWindow[] }

/**
 * An album of the project's gallery, or of the PowerGrades that every project shares (p. 3266), or
 * the project's one album of the memories A–H (pp. 3271–3272), which `gallery.list` gives apart.
 */
export type AlbumKind = 'stills' | 'powergrade' | 'memories';
/** Which grade of its clip a still holds: the Node Editor's mode when it was grabbed (p. 3306). */
export type StillLevel = 'clip' | 'pre_clip' | 'post_clip';
/** Apply Grades Using (pp. 3264, 3287): how the keys of a still's grade are put on a clip. */
export type KeyframesUsing = 'none' | 'source_timecode' | 'start_frame';

/**
 * A still of the Gallery as `gallery.list` gives it (PROTOCOL.md, "What a still holds"): its
 * number Track.Shot.Version (`name`), its label, and where it came from, without its grades.
 */
export interface Still {
  id: string;
  album: string;
  name: string;
  label: string;
  /** When it was grabbed or imported, RFC 3339 in UTC. */
  created: string;
  image: { file: string; format: 'png' | 'exr'; width: number; height: number };
  level: StillLevel;
  /** Whether it holds a clip grade and a timeline grade. */
  grades: { clip: boolean; timeline: boolean };
  source: StillSource | null;
  /** Of an imported still, the path it was taken from. */
  imported?: string;
}
/** Where a still came from (Still Properties, p. 3258); null of an imported picture. */
export interface StillSource {
  timeline: { id: string; name: string; rate: Rate };
  frame: number;
  record_timecode: string;
  clip?: { id: string } | null;
  media?: { id: string; name?: string } | null;
  group?: string | null;
  track?: number | null;
  shot?: number | null;
  source_frame?: number | null;
  source_timecode?: string | null;
}
/** A still as `still.get` gives it: with its grades, as they are and frozen, and the colour of the project then. */
export interface StillWhole extends Omit<Still, 'grades' | 'source'> {
  grades: { clip: Grade | null; timeline: Grade | null };
  frozen: { clip: Grade | null; timeline: Grade | null };
  source: (StillSource & { clip?: (Omit<Clip, 'grade'> & { name?: string }) | null; media?: Media | null }) | null;
}
/** An album, with its stills in the order of their ids (Sort Stills › Still ID, p. 3263). */
export interface GalleryAlbum { id: string; name: string; kind: AlbumKind; stills: Still[] }
/** What `still.import` answers: the stills added. The shell asks for the files in the dialog of the system. */
export interface StillsImported { added: Still[] }
/** What `still.export` answers: the files written for each still. The shell asks for the place in Save As. */
export interface StillsExported { written: { image: string; json: string }[] }
/** What `grade.export_lut` answers (Generate LUT): where the cube is, its name in the library where it lies there, and the nodes baked and left out. */
export interface LutExported { path: string; name: string | null; size: number; nodes: string[]; ignored: { node: string; why: string }[] }

/** The styles of a wipe that the engine draws (p. 3000); Alpha is not done. */
export type WipeStyle = 'horizontal' | 'vertical' | 'diagonal' | 'mix' | 'difference' | 'box' | 'venetian_blind' | 'checkerboard';
/**
 * What the viewer compares its frame with (PROTOCOL.md, "Comparisons in the viewer"): a still of
 * the Gallery by its id (the page names no file), the timeline at a frame or a clip, the clip
 * `steps` (1–7, 1 where left out) before or after the current one following the playhead, or (of a
 * split screen) the frame shown or a black cell.
 */
export type ViewerReference =
  | { kind: 'still'; still: string }
  | { kind: 'frame'; frame: number }
  | { kind: 'clip'; clip: string; frame?: number }
  | { kind: 'neighbour'; side: 'previous' | 'next'; steps?: number; current?: string }
  | { kind: 'current' }
  | { kind: 'black' };
/** A reference as the engine answers it: of the timeline, with the frame it shows now (`at`). */
export type ComparedReference = ViewerReference & { at?: number | null; path?: string };
/** The whole of a wipe: every setting is sent each time, what is left out taking its default. */
export interface WipeSettings {
  style: WipeStyle;
  position: number;
  angle: number;
  invert: boolean;
  softness?: number;
  border?: number;
  border_color?: [number, number, number];
  gain?: number;
}
/** What `viewer.wipe` and `viewer.split` answer: what the viewer compares now. */
export type Compared =
  | { mode: 'off' }
  | { mode: 'wipe'; wipe: WipeSettings & { on: boolean; reference: ComparedReference } }
  | { mode: 'split'; split: { on: boolean; items: ComparedReference[]; selected?: number; outline: boolean } };

/**
 * The graticule of a scope (PROTOCOL.md, "Scopes"): its lines and labels in pixels of its picture
 * from the top left corner, a label written from `at`, its left at the middle of its height.
 */
export interface ScopeGraticule {
  width: number; height: number;
  lines: { from: [number, number]; to: [number, number]; colour: [number, number, number] }[];
  labels: { at: [number, number]; text: string; colour: [number, number, number] }[];
}
/** What `scopes.subscribe` and `scopes.set` answer: the scope, the size of its pictures, its graticule and its settings whole. */
export interface ScopeAnswer {
  scope_id: string;
  size: { width: number; height: number };
  graticule: ScopeGraticule;
  settings: Record<string, unknown>;
}
/** A picture of a scope, `scopes.frame`: RGBA of 8 bits by rows from the top, as base64. */
export interface ScopeFrame {
  scope_id: string; view: Monitor; frame: number; width: number; height: number; rgba: string;
}

/**
 * The gesture that a transaction is part of (`transaction.apply` with `gesture`): a control
 * dragged, sent as it goes. The engine applies and draws each, keeps them all as one step to undo,
 * and takes a `revision` of any since the gesture began; `end` on the last ends it (PROTOCOL.md,
 * "Gestures").
 */
export interface TransactionGesture { id: string; end?: boolean }

/** Where at a node of a grade a picture is taken: what goes into it, what comes out, or its key (as Highlight BW shows it). */
export type NodePlace = 'input' | 'output' | 'key';

/**
 * What `view.sample` is asked: a point of the picture that a monitor shows (`monitor`), or of a
 * frame of a timeline drawn off screen (`timeline` and `frame`), in pixels of the frame from its
 * top left corner (as `view.pointer` tells them), the mean of a square of `size` pixels about it
 * (odd, 1 to 31); and, with `grade` and `node`, the colour that goes into the node there and
 * comes out of it.
 */
export interface ViewSampleParams {
  monitor?: Monitor;
  timeline?: string;
  frame?: number;
  x: number;
  y: number;
  size?: number;
  grade?: Target;
  node?: string;
  /** For a grade of a group: the clip whose picture is taken, rather than the one on top. */
  clip?: string;
}

/** The answer to `view.sample`: R'G'B' of the frame as it is shown, and of the node's input and output. */
export interface ViewSample {
  monitor?: Monitor;
  timeline?: string;
  frame: number | null;
  x: number;
  y: number;
  size: number;
  width: number;
  height: number;
  region: { x: number; y: number; width: number; height: number };
  rgb: [number, number, number];
  /** `key`: the node's key there, 0…1, as Highlight BW shows it. */
  node?: { grade: Target; node: string; input: [number, number, number]; output: [number, number, number]; key: number };
}

/** The cursor that a view that takes the pointer shows. */
export type PointerCursor = 'arrow' | 'crosshair' | 'move' | 'resize_ew' | 'resize_ns';

/**
 * The notification `view.pointer`: what the pointer did on a view that takes it (`view.pointer`
 * with `on`), for a tool of the page used on the picture. `x` and `y` are pixels of the frame
 * shown (of the timeline, for the timeline monitor), null where none is drawn; a drag that goes
 * beyond the picture goes on, `inside` false.
 */
export type ViewerTool = 'none' | 'qualifier' | 'window' | 'image_wipe';
/** A Linear corner or Polygon point dragged by a control: the left key frame of each coordinate. */
/** A point's left keys: its position, and for a Curve its Bezier handles (`in_*`, `out_*`). */
export interface WindowPointKeys {
  point: number; x?: number | null; y?: number | null;
  in_x?: number | null; in_y?: number | null; out_x?: number | null; out_y?: number | null;
}
/**
 * The left keys a captured control edits (PROTOCOL.md). `edge` names the Linear side whose Soft 1–4
 * is `softness`; `inserted` is a Polygon or Curve point added by this capture, whose keys are in `points`;
 * `inside`/`outside` are a Curve's softness dragged on screen.
 */
export interface WindowKeys {
  size?: number | null; aspect?: number | null; pan?: number | null; tilt?: number | null; rotate?: number | null; softness?: number | null;
  edge?: number; points?: WindowPointKeys[]; inserted?: number; inside?: number | null; outside?: number | null;
  clock: 'source' | 'clip' | 'timeline'; frame: number;
}
/** Why a whole-animation Polygon or Curve point insertion or deletion was refused; nothing changed. */
export type WindowTopology =
  | { why: 'maximum'; most: number } | { why: 'minimum'; least: number; shape?: 'polygon' | 'curve' } | { why: 'unsupported' }
  | { why: 'jump' | 'timing' | 'overshoot' | 'range' | 'encoding'; axis: 'x' | 'y'; points: number[];
    frames: [number, number] | null; clock: 'source' | 'clip' | 'timeline' };
/** The answer of `window.input` with `kind: "delete"` (a native middle-button tap). */
export interface WindowDeleted { binding: number; phase: 'deleted'; selected: string; point: number; revision: number }
export interface WindowState {
  binding: number; capture: number; sequence: number; phase: 'arming' | 'armed' | 'refused' | 'ended';
  reason: string | null; selected: string; event: number; revision: number; keys: WindowKeys;
  /** The selected Curve's Shift-selected anchors (0-based) as this state is sent; empty for other shapes. */
  selected_points: number[];
}
/** The answer of `window.input` with `kind: "delete_points"` (Backspace on a Curve's selected points). */
export interface WindowPointsDeleted { binding: number; phase: 'deleted'; selected: string; points: number[]; revision: number }
/** The answer of `window.input` with `kind: "clear_points"` (Esc): how many points it deselected. */
export interface WindowPointsCleared { binding: number; phase: 'cleared'; cleared: number }
export interface WindowUnbound { binding: number; sequence: number; revision: number; reason: 'project_changed' }
/** `window.points`: a project change (undo, redo, another client's edit) dropped the selected Curve's point selection. */
export interface WindowPointsDropped { binding: number; sequence: number; revision: number; selected_points: [] }
export interface WindowSelection {
  monitor: 'timeline'; revision: number; timeline: string; target: Target; clip?: string;
  node: string; selected: string | null; visible: boolean; outline: boolean; tool: ViewerTool;
}
export interface ViewPointer {
  /** Native event evidence retained by the server; clients never synthesize coordinates for Window. */
  event?: number;
  monitor: Monitor;
  /** Engine-local native attachment identity; old queued events keep their old instance. */
  instance?: number;
  /** Successful native presentation copied with the pointer coordinates. */
  presentation?: {
    serial: number;
    tag: number | null;
    map: { surface: [number, number]; picture: [number, number]; dest: [number, number, number, number] } | null;
  } | null;
  /** Present only when this instance/tag resolves to a retained timeline frame. */
  snapshot?: {
    revision: number;
    timeline: string;
    frame: number;
    canvas: [number, number];
    ready: boolean;
    editable: boolean;
    comparison: 'off' | 'wipe' | 'split';
    window?: { binding: number; selected: string | null } | null;
  } | null;
  kind: 'down' | 'move' | 'up' | 'cancel' | 'wheel';
  /** Raw signed wheel units (120 per detent), including high precision +/-30 input. */
  delta?: number;
  /** Native double-click is the second down; no renderer timer synthesizes it. */
  clicks?: number;
  middle_moved?: boolean;
  button: 'left' | 'right' | 'middle' | null;
  buttons: ('left' | 'right' | 'middle')[];
  x: number | null;
  y: number | null;
  inside: boolean;
  window: { x: number; y: number };
  shift: boolean;
  ctrl: boolean;
  alt: boolean;
}

/** The pointer on a view, where an event of the engine tells it. */
export function viewPointerOf(event: EngineEvent): ViewPointer | null {
  return event.kind === 'notification' && event.method === 'view.pointer' && event.params !== null && typeof event.params === 'object'
    ? event.params as ViewPointer : null;
}

/** What `scopes.statistics` is asked, besides `bins`: the frame a monitor shows, or a frame of a timeline drawn off screen; and, with `grade` and `node`, the picture at the node. */
export interface ScopeStatisticsParams {
  view?: Monitor;
  timeline?: string;
  frame?: number;
  bins?: number;
  grade?: Target;
  node?: string;
  at?: NodePlace;
  clip?: string;
}

/** A channel's figures in `scopes.statistics`: its least, greatest and mean signal (0 to 1, or beyond). */
export interface ChannelFigures { min: number; max: number; mean: number }
/** The figures of the frame that a monitor shows now (PROTOCOL.md, "Scopes"): the Primaries palette's Auto Color reads them. */
export interface ScopeStatistics {
  /** The monitor measured; left out for a frame of a timeline drawn off screen. */
  view?: Monitor; frame: number; width: number; height: number; samples: number;
  /** The timeline of the frame, where it was named or measured at a node. */
  timeline?: string;
  /** The node at which the picture was taken. */
  node?: { grade: Target; node: string; at: NodePlace };
  red: ChannelFigures; green: ChannelFigures; blue: ChannelFigures; luma: ChannelFigures;
  chroma: { centroid: [number, number]; spread: [number, number]; radius: number; saturation: number; hue: number };
  /** The signal below which 1%, 50% and 99% of the pixels are. */
  percentiles: Record<'red' | 'green' | 'blue' | 'luma', { '1': number; '50': number; '99': number }>;
  histogram: { bins: number; red: number[]; green: number[]; blue: number[]; luma: number[] };
}

/** Where the viewer lies: device pixels of the screen, from the top left corner of the client area. */
export interface ViewRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** What the shell answers when a project is opened in the studio. */
export interface EngineOpened {
  /** The number of this engine's session; every event of it carries it. */
  session: number;
  /** The directory of the project of the studio. */
  project: string;
  /** Whether this application started the engine, or reached one that had the project open. */
  owned: boolean;
  pid: number;
  revision: number;
}

/** What the shell tells the page of the engine, in the order it happened. */
export type EngineEvent =
  | ({ kind: 'opened' } & EngineOpened)
  | { kind: 'notification'; session: number; method: string; params: unknown }
  | { kind: 'exited'; session: number; expected: boolean; code: number | null; signal: string | null }
  | { kind: 'error'; session: number; message: string; code?: number | null; data?: unknown }
  | { kind: 'attached'; session: number; monitor: Monitor; adapter: string | null; backend: string | null }
  | { kind: 'unsupported'; session: number; monitor: Monitor; reason: 'unsupported' | 'headless'; message: string }
  /** A headless engine (an agent's) had the project, and was asked to yield it; `opened` follows. */
  | { kind: 'taking-over'; session: number; pid: number | null }
  /**
   * The old projects of the folder were looked at once the engine was open: those that became
   * timelines and those that could not, or why none could be looked at (`error`). Told only where
   * there was something to say.
   */
  | { kind: 'migrated'; session: number; migrations: MigrationReport[]; error: string | null }
  /**
   * The pixels of the screen that the viewer covers changed (the page was zoomed): the page
   * measures the viewer again and says where it is. Told with no session where none is open.
   */
  | { kind: 'measure'; session: number | null }
  /** The floating window of the video scopes was closed by itself (its own button or keys): the page shows its scopes docked again. */
  | { kind: 'scopes-window'; open: boolean; session: number | null };

/** A failure as the engine gave it, or one of the shell's own (code null). */
export interface EngineFailure {
  code: number | null;
  message: string;
  data: unknown;
}

/** What the shell answers to a call of the engine: what the engine answered, or its error whole. */
export type EngineAnswer<T = unknown> = { ok: true; result: T } | { ok: false; error: EngineFailure };

/** An error of the engine, with its code and data: `-32001` for a refused transaction, and so on. */
export class EngineError extends Error {
  readonly code: number | null;
  readonly data: unknown;
  constructor(failure: EngineFailure) {
    super(failure.message);
    this.name = 'EngineError';
    this.code = failure.code;
    this.data = failure.data;
  }
}

/** Call a method of the engine, and get its result, or an `EngineError`. */
export async function callEngine<M extends PageMethod>(
  bridge: Pick<EngineBridge, 'call'>, method: M, params?: Record<string, unknown>,
  context?: EngineCallContext,
): Promise<PageMethods[M]> {
  const answer = await (context === undefined ? bridge.call(method, params) : bridge.call(method, params, context));
  if (!answer.ok) throw new EngineError(answer.error);
  return answer.result as PageMethods[M];
}

/**
 * The rectangle of an element in device pixels, as the engine takes it: its left and top rounded,
 * then its right and bottom, and the width and height between them, so that neighbours share
 * their edges. `box` is `getBoundingClientRect()`, and `ratio` is `devicePixelRatio`.
 */
export function deviceRect(box: { left: number; top: number; right: number; bottom: number }, ratio: number): ViewRect {
  const x = Math.max(0, Math.round(box.left * ratio));
  const y = Math.max(0, Math.round(box.top * ratio));
  const right = Math.max(x, Math.round(box.right * ratio));
  const bottom = Math.max(y, Math.round(box.bottom * ratio));
  return { x, y, width: right - x, height: bottom - y };
}

/** A copy that does not hold what the engine says it changed: it is fetched anew. */
export class CopyMismatch extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CopyMismatch';
  }
}

function placeOf<T extends { id: string }>(list: readonly T[], id: string, what: string): number {
  const at = list.findIndex(thing => thing.id === id);
  if (at < 0) throw new CopyMismatch(`there is no ${what} ${id} in the copy`);
  return at;
}

function inserted<T>(list: readonly T[], thing: T, index: number | null | undefined): T[] {
  const at = index ?? list.length;
  if (!Number.isInteger(at) || at < 0 || at > list.length) throw new CopyMismatch(`index ${at} is past the end of a list of ${list.length}`);
  return [...list.slice(0, at), thing, ...list.slice(at)];
}

function without<T>(list: readonly T[], at: number): T[] {
  return [...list.slice(0, at), ...list.slice(at + 1)];
}

function replaced<T>(list: readonly T[], at: number, thing: T): T[] {
  return list.map((each, index) => (index === at ? thing : each));
}

/** Put a clip among the clips of a track, in the order of their start: before the first that starts no earlier. */
function put(clips: readonly Clip[], clip: Clip): Clip[] {
  const at = clips.findIndex(each => !(each.start < clip.start));
  return inserted(clips, clip, at < 0 ? clips.length : at);
}

/** The engine sorts clips/cues by start, transitions by their right clip's cut. */
function ordered(track: Track): Track {
  if (track.kind === 'subtitle') return { ...track, cues: [...track.cues].sort((a, b) => a.start - b.start) };
  const starts = new Map(track.clips.map(clip => [clip.id, clip.start]));
  return { ...track, clips: [...track.clips].sort((a, b) => a.start - b.start),
    transitions: [...track.transitions].sort((a, b) => (starts.get(a.right) ?? Infinity) - (starts.get(b.right) ?? Infinity)) };
}

function clipAt(timeline: Timeline, id: string): [number, number] {
  for (let index = 0; index < timeline.tracks.length; index += 1) {
    const track = timeline.tracks[index];
    if (track.kind === 'subtitle') continue;
    const at = track.clips.findIndex(clip => clip.id === id);
    if (at >= 0) return [index, at];
  }
  throw new CopyMismatch(`there is no clip ${id} in the copy`);
}

function inTimeline(project: Project, id: string, change: (timeline: Timeline) => Timeline): Project {
  const at = placeOf(project.timelines, id, 'timeline');
  return { ...project, timelines: replaced(project.timelines, at, change(project.timelines[at])) };
}

function inTrack(timeline: Timeline, at: number, change: (track: Track) => Track): Timeline {
  return { ...timeline, tracks: replaced(timeline.tracks, at, change(timeline.tracks[at])) };
}

function clipTrack(track: Track): ClipTrack {
  if (track.kind === 'subtitle') throw new CopyMismatch(`track ${track.id} holds subtitles, not clips`);
  return track;
}

function inClips(timeline: Timeline, at: number, change: (clips: Clip[]) => Clip[]): Timeline {
  return inTrack(timeline, at, track => ordered({ ...clipTrack(track), clips: change(clipTrack(track).clips) }));
}

/** Only double-option fields distinguish null (clear it) from an absent field (keep it). */
function patched<T>(item: T, op: object, keys: readonly string[], nullable: readonly string[] = []): T {
  const fields = op as Record<string, unknown>;
  const changed = Object.fromEntries(keys.filter(key => fields[key] !== undefined && (fields[key] !== null || nullable.includes(key))).map(key => [key, fields[key]]));
  return { ...item, ...changed };
}

function inCue(timeline: Timeline, id: string, change: (cues: Cue[], at: number) => Cue[]): Timeline {
  for (let i = 0; i < timeline.tracks.length; i += 1) {
    const track = timeline.tracks[i];
    if (track.kind !== 'subtitle') continue;
    const at = track.cues.findIndex(cue => cue.id === id);
    if (at >= 0) return inTrack(timeline, i, () => ordered({ ...track, cues: change(track.cues, at) }));
  }
  throw new CopyMismatch(`there is no subtitle ${id} in the copy`);
}

function inTransition(timeline: Timeline, id: string, change: (items: Transition[], at: number) => Transition[]): Timeline {
  for (let i = 0; i < timeline.tracks.length; i += 1) {
    const track = timeline.tracks[i];
    if (track.kind === 'subtitle') continue;
    const at = track.transitions.findIndex(item => item.id === id);
    if (at >= 0) return inTrack(timeline, i, () => ordered({ ...track, transitions: change(track.transitions, at) }));
  }
  throw new CopyMismatch(`there is no transition ${id} in the copy`);
}

/**
 * A grade as the engine keeps it: one left out (as a client of schema 2 builds a clip or a
 * timeline) is read as the empty grade, and links are put in their order.
 */
function kept(grade: Grade | undefined): Grade {
  return grade === undefined ? emptyGrade() : { ...grade, links: sortLinks(grade.links) };
}

/** A clip as the engine keeps it, its grade and group read as `kept` reads them. */
function keptClip(clip: Clip): Clip {
  return { ...clip, grade: kept(clip.grade), group: clip.group ?? null };
}

function keptTrack(track: Track): Track {
  return ordered(track.kind === 'subtitle' ? track : { ...track, clips: track.clips.map(keptClip) });
}

function keptTimeline(timeline: Timeline): Timeline {
  return { ...timeline, grade: kept(timeline.grade), tracks: timeline.tracks.map(keptTrack) };
}

function inGroup(project: Project, id: string, change: (group: Group) => Group): Project {
  const at = placeOf(project.groups, id, 'group');
  return { ...project, groups: replaced(project.groups, at, change(project.groups[at])) };
}

function inClip(timeline: Timeline, id: string, change: (clip: Clip) => Clip): Timeline {
  const [track, at] = clipAt(timeline, id);
  return inTrack(timeline, track, each => {
    const clips = clipTrack(each).clips;
    return { ...clipTrack(each), clips: replaced(clips, at, change(clips[at])) };
  });
}

/** Change the grade that `target` names; its links are put in order after. */
function inGrade(project: Project, target: Target, change: (grade: Grade) => Grade): Project {
  const sorted = (grade: Grade): Grade => {
    const changed = change(grade);
    return { ...changed, links: sortLinks(changed.links) };
  };
  switch (target.level) {
    case 'clip': return inTimeline(project, target.timeline, timeline => inClip(timeline, target.clip, clip => ({ ...clip, grade: sorted(clip.grade) })));
    case 'timeline': return inTimeline(project, target.timeline, timeline => ({ ...timeline, grade: sorted(timeline.grade) }));
    case 'pre_clip': return inGroup(project, target.group, group => ({ ...group, pre: sorted(group.pre) }));
    case 'post_clip': return inGroup(project, target.group, group => ({ ...group, post: sorted(group.post) }));
    default: throw new CopyMismatch(`the grade of level ${(target as { level: unknown }).level} is not known to the copy`);
  }
}

const sameInput = (a: Pick<Link, 'to' | 'kind' | 'input'>, b: Pick<Link, 'to' | 'kind' | 'input'>): boolean =>
  a.to === b.to && a.kind === b.kind && a.input === b.input;

/** Put links into a grade, each in place of the link its input had. */
function linked(links: readonly Link[], added: readonly Link[]): Link[] {
  let all = [...links];
  for (const link of added) all = [...all.filter(each => !sameInput(each, link)), link];
  return all;
}

function nodeAt(grade: Grade, id: string): number {
  const at = grade.nodes.findIndex(node => node.id === id);
  if (at < 0) throw new CopyMismatch(`there is no node ${id} in the grade of the copy`);
  return at;
}

/** How many inputs of `kind` the node or end `id` has (`Grade::inputs_of`). */
function inputsOf(grade: Grade, id: string, kind: LinkKind): number {
  if (id === 'output') return kind === 'rgb' ? 1 : 0;
  if (id === 'source') return 0;
  const node = grade.nodes.find(each => each.id === id);
  if (node === undefined) return 0;
  switch (node.kind) {
    case 'corrector': return 1;
    case 'parallel_mixer': case 'layer_mixer': return kind === 'rgb' ? node.params.inputs : 0;
    case 'key_mixer': return kind === 'key' ? node.params.inputs.length : 0;
    default: return 0;
  }
}

/**
 * The links that take the place of a node taken away, as DaVinci links the nodes on either side
 * (`Grade::bridge`): what comes into its first linked input of each kind goes into every input its
 * output of that kind went into. The engine's `done` carries them; this is for a `remove_node`
 * without them.
 */
function bridge(grade: Grade, id: string): Link[] {
  const links: Link[] = [];
  for (const kind of ['rgb', 'key'] as const) {
    let before: Link | undefined;
    for (let input = 0; input < inputsOf(grade, id, kind) && before === undefined; input += 1) {
      before = grade.links.find(link => sameInput(link, { to: id, kind, input }));
    }
    if (before === undefined) continue;
    for (const after of grade.links) {
      if (after.from === id && after.kind === kind) links.push({ from: before.from, to: after.to, kind, input: after.input });
    }
  }
  return links;
}

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);

/**
 * Put `patch` into `base` as the engine does (`grade::merge`): where both are objects, each field of
 * the patch into the field of that name; otherwise the patch in place of the base, whole. A null
 * is a value like any other.
 */
export function mergePatch(base: unknown, patch: unknown): unknown {
  if (!isObject(base) || !isObject(patch)) return patch;
  const merged: Record<string, unknown> = { ...base };
  for (const [name, value] of Object.entries(patch)) merged[name] = Object.hasOwn(base, name) ? mergePatch(base[name], value) : value;
  return merged;
}

function setNode(node: GradeNode, op: Extract<Op, { op: 'set_node' }>): GradeNode {
  // `pos: null` puts a node back where the editor lays it out; left out, it stays.
  const changed = patched(node, op, ['label', 'enabled', 'locked', 'pos'], ['pos']);
  return op.params == null ? changed : { ...changed, params: mergePatch(changed.params, op.params) } as GradeNode;
}

/** Mirror only accepted basic operations, preserving all objects outside the changed branch. */
export function applyOp(project: Project, op: Op): Project {
  switch (op.op) {
    case 'add_media': return { ...project, media: inserted(project.media, op.media, op.index) };
    case 'remove_media': return { ...project, media: without(project.media, placeOf(project.media, op.id, 'media')) };
    case 'set_media': {
      const at = placeOf(project.media, op.id, 'media');
      let media = patched(project.media[at], op, ['name', 'bin', 'mark_in', 'mark_out'], ['bin', 'mark_in', 'mark_out']);
      if (op.still != null) {
        if (!media.facts.video) throw new CopyMismatch(`media ${op.id} has no picture`);
        const { still: _old, ...picture } = media.facts.video;
        media = { ...media, facts: { ...media.facts, video: op.still ? { ...picture, still: true } : picture } };
      }
      return { ...project, media: replaced(project.media, at, media) };
    }
    case 'set_proxy': {
      const at = placeOf(project.media, op.media, 'media');
      const { proxy: _before, ...media } = project.media[at];
      return { ...project, media: replaced(project.media, at, op.proxy == null ? media : { ...media, proxy: op.proxy }) };
    }
    case 'add_bin': return { ...project, bins: inserted(project.bins, op.bin, op.index) };
    case 'remove_bin': return { ...project, bins: without(project.bins, placeOf(project.bins, op.id, 'bin')) };
    case 'set_bin': {
      const at = placeOf(project.bins, op.id, 'bin');
      return { ...project, bins: replaced(project.bins, at, patched(project.bins[at], op, ['name', 'parent'], ['parent'])) };
    }
    case 'add_timeline': return { ...project, timelines: inserted(project.timelines, keptTimeline(op.timeline), op.index) };
    case 'remove_timeline': return { ...project, timelines: without(project.timelines, placeOf(project.timelines, op.id, 'timeline')) };
    case 'set_timeline': return inTimeline(project, op.id, timeline => patched(timeline, op, ['name', 'rate', 'width', 'height', 'sample_rate']));
    case 'add_track': return inTimeline(project, op.timeline, timeline => ({ ...timeline, tracks: inserted(timeline.tracks, keptTrack(op.track), op.index) }));
    case 'remove_track': return inTimeline(project, op.timeline, timeline => ({ ...timeline, tracks: without(timeline.tracks, placeOf(timeline.tracks, op.id, 'track')) }));
    case 'set_track': return inTimeline(project, op.timeline, timeline => inTrack(timeline, placeOf(timeline.tracks, op.id, 'track'), track => patched(track, op, ['name', 'enabled', 'locked', ...(track.kind === 'subtitle' ? ['language'] : [])])));
    case 'insert_clip': return inTimeline(project, op.timeline, timeline => inClips(timeline, placeOf(timeline.tracks, op.track, 'track'), clips => put(clips, keptClip(op.clip))));
    case 'remove_clip': return inTimeline(project, op.timeline, timeline => {
      const [track, at] = clipAt(timeline, op.id);
      return inClips(timeline, track, clips => without(clips, at));
    });
    case 'move_clip': return inTimeline(project, op.timeline, timeline => {
      const [from, at] = clipAt(timeline, op.id);
      const to = placeOf(timeline.tracks, op.track, 'track');
      const moved = { ...clipTrack(timeline.tracks[from]).clips[at], start: op.start };
      const taken = inClips(timeline, from, clips => without(clips, at));
      return inClips(taken, to, clips => put(clips, moved));
    });
    case 'shift_clips': return inTimeline(project, op.timeline, timeline => {
      const at = placeOf(timeline.tracks, op.track, 'track');
      const ids = new Set(op.clips);
      return inClips(timeline, at, clips => {
        for (const id of ids) placeOf(clips, id, 'clip');
        return clips.map(clip => ids.has(clip.id) ? { ...clip, start: clip.start + op.by } : clip);
      });
    });
    case 'trim_clip': return inTimeline(project, op.timeline, timeline => {
      const [track, at] = clipAt(timeline, op.id);
      return inClips(timeline, track, clips => {
        const clip = clips[at];
        const source = clip.source.kind === 'media' && op.source_in != null ? { ...clip.source, in: op.source_in } : clip.source;
        return put(without(clips, at), { ...clip, source, start: op.start, length: op.length });
      });
    });
    case 'set_clip': return inTimeline(project, op.timeline, timeline => {
      const [track, at] = clipAt(timeline, op.id);
      return inClips(timeline, track, clips => replaced(clips, at, patched(clips[at], op,
        ['enabled', 'speed', 'reverse', 'link', 'transform', 'gain_db', 'pan', 'fade_in', 'fade_out', 'source'], ['link'])));
    });
    case 'add_transition': return inTimeline(project, op.timeline, timeline => inTrack(timeline, placeOf(timeline.tracks, op.track, 'track'), track => {
      const to = clipTrack(track);
      return ordered({ ...to, transitions: [...to.transitions, op.transition] });
    }));
    case 'remove_transition': return inTimeline(project, op.timeline, timeline => inTransition(timeline, op.id, (items, at) => without(items, at)));
    case 'set_transition': return inTimeline(project, op.timeline, timeline => inTransition(timeline, op.id, (items, at) => replaced(items, at, patched(items[at], op, ['kind', 'length', 'align']))));
    case 'add_cue': return inTimeline(project, op.timeline, timeline => inTrack(timeline, placeOf(timeline.tracks, op.track, 'track'), track => {
      if (track.kind !== 'subtitle') throw new CopyMismatch(`track ${track.id} holds no subtitles`);
      return ordered({ ...track, cues: [...track.cues, op.cue] });
    }));
    case 'shift_cues': return inTimeline(project, op.timeline, timeline => inTrack(timeline, placeOf(timeline.tracks, op.track, 'track'), track => {
      if (track.kind !== 'subtitle') throw new CopyMismatch(`track ${track.id} holds no subtitles`);
      const ids = new Set(op.cues);
      for (const id of ids) placeOf(track.cues, id, 'cue');
      return ordered({ ...track, cues: track.cues.map(cue => ids.has(cue.id) ? { ...cue, start: cue.start + op.by } : cue) });
    }));
    case 'remove_cue': return inTimeline(project, op.timeline, timeline => inCue(timeline, op.id, (items, at) => without(items, at)));
    case 'set_cue': return inTimeline(project, op.timeline, timeline => inCue(timeline, op.id, (items, at) => replaced(items, at, patched(items[at], op, ['start', 'length', 'text']))));
    case 'add_node': return inGrade(project, op.grade, grade => ({
      nodes: inserted(grade.nodes, op.node, op.index),
      links: linked(grade.links, op.links ?? []),
    }));
    case 'remove_node': return inGrade(project, op.grade, grade => {
      const at = nodeAt(grade, op.id);
      const links = op.links ?? bridge(grade, op.id);
      return { nodes: without(grade.nodes, at), links: [...grade.links.filter(link => link.from !== op.id && link.to !== op.id), ...links] };
    });
    case 'connect': return inGrade(project, op.grade, grade => ({ ...grade, links: linked(grade.links, [op.link]) }));
    case 'disconnect': return inGrade(project, op.grade, grade => {
      const at = grade.links.findIndex(link => sameInput(link, op));
      if (at < 0) throw new CopyMismatch(`there is no link into input ${op.input} of ${op.kind} of ${op.to} in the copy`);
      return { ...grade, links: without(grade.links, at) };
    });
    case 'set_node': return inGrade(project, op.grade, grade => {
      const at = nodeAt(grade, op.id);
      return { ...grade, nodes: replaced(grade.nodes, at, setNode(grade.nodes[at], op)) };
    });
    case 'set_grade': return inGrade(project, op.grade, () => op.value);
    case 'add_group': return { ...project, groups: inserted(project.groups, { ...op.group, pre: kept(op.group.pre), post: kept(op.group.post) }, op.index) };
    case 'remove_group': return { ...project, groups: without(project.groups, placeOf(project.groups, op.id, 'group')) };
    case 'set_group': return inGroup(project, op.id, group => patched(group, op, ['name']));
    case 'set_clip_group': return inTimeline(project, op.timeline, timeline => inClip(timeline, op.clip, clip => ({ ...clip, group: op.group ?? null })));
    case 'set_color_settings': return { ...project, color: mergePatch(keptColor(project), op.settings) as ColorSettings };
    case 'set_media_color': {
      const at = placeOf(project.media, op.id, 'media');
      const media = project.media[at];
      // A pair of LUTs is set whole, a LUT left out as none, as the engine reads it.
      const set = op.lut ? { ...op, lut: { '1d': op.lut['1d'] ?? null, '3d': op.lut['3d'] ?? null } } : op;
      const override = patched(keptOverride(media), set, ['input', 'levels', 'lut', 'inverse_drt'], ['input']);
      return { ...project, media: replaced(project.media, at, { ...media, color_override: override }) };
    }
    default: throw new CopyMismatch(`the operation ${(op as { op: unknown }).op} is not known to the copy`);
  }
}

/** Apply a change that the engine told of: its `next_id` first, then each of its operations in order. */
export function applyChange(project: Project, change: Pick<ProjectChanged, 'done' | 'next_id'>): Project {
  let changed: Project = { ...project, next_id: change.next_id };
  for (const op of change.done) changed = applyOp(changed, op);
  return changed;
}

/** Whether the copy is kept by an engine that answers. */
export type CopyStatus = 'connected' | 'disconnected';

/** How the view of the engine stands in the window. */
export type ViewState =
  | { state: 'none' }
  | { state: 'attached'; adapter: string | null; backend: string | null }
  | { state: 'unsupported'; reason: 'unsupported' | 'headless' };

/** What the page knows of the engine at one moment. A new object whenever anything in it changed. */
export interface StudioSnapshot {
  status: CopyStatus;
  /** The session of the engine that the copy was fetched from, or null before any was opened. */
  session: number | null;
  /** The revision of the copy, or null before it was fetched. */
  revision: number | null;
  /**
   * The copy of the project, kept (as it was last) while disconnected; null before it was fetched,
   * and from the moment another engine is opened until its project is.
   */
  project: Project | null;
  /** What the transport told last, or null before it told anything. */
  transports: { source: SourceTransportState | null; timeline: TimelineTransportState | null };
  /** How the proxy of each media item is being made, as the engine told last, by the media. */
  proxies: Readonly<Record<string, ProxyEvent>>;
  views: Record<Monitor, ViewState>;
  /** What went wrong last: an error of the engine, or a fetch that failed. Null where nothing did. */
  error: string | null;
  /** Whether the headless engine of an agent is being asked to yield the project, until it is opened. */
  takingOver: boolean;
}

const NOTHING: StudioSnapshot = {
  status: 'disconnected', session: null, revision: null, project: null, transports: { source: null, timeline: null }, proxies: {}, views: { source: { state: 'none' }, timeline: { state: 'none' } },
  error: null, takingOver: false,
};

/** How long the copy waits before it fetches the project again after a fetch failed: twice as long each time, up to 5 s. */
const RETRY_FIRST_MS = 250;
const RETRY_MOST_MS = 5_000;

/**
 * The page's copy of the project, kept as the engine tells it.
 *
 * It fetches the project with `project.get` whenever an engine is opened (also again, as after the
 * page was loaded again), and then applies each `project.changed` whose revision is one above its
 * own, or whose `base` is its own (the changes of a gesture, told together). One at or below its own is passed over; one that skips a revision, or that the copy cannot
 * apply, makes it fetch the project again. What is told while it fetches is applied after the
 * fetch, from the revision that the fetch answered. A fetch that fails is tried again, a little
 * later each time, while the engine is open. It also keeps what the transport, the proxies and the
 * view told last.
 *
 * It is a store for `useSyncExternalStore(copy.subscribe, copy.getSnapshot)`.
 */
export class ProjectCopy {
  private snapshot: StudioSnapshot = NOTHING;
  private readonly listeners = new Set<() => void>();
  private fetching: Promise<void> | null = null;
  private fetchAgain = false;
  /** What was told while the project was being fetched. */
  private held: ProjectChanged[] = [];
  /** Counted up whenever an engine is opened or ends, so that a fetch of an earlier one is let go. */
  private epoch = 0;
  /** Whether an engine is open, as the shell told: one was opened, and has not ended. */
  private open = false;
  /** How often a fetch failed in a row, and the fetch that is waited for after it. */
  private failures = 0;
  private retry: ReturnType<typeof setTimeout> | null = null;
  private readonly unsubscribe: () => void;

  constructor(private readonly bridge: Pick<EngineBridge, 'call' | 'onEvent'>) {
    this.unsubscribe = bridge.onEvent(event => this.heard(event));
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  readonly getSnapshot = (): StudioSnapshot => this.snapshot;

  /** Stop listening to the engine. The snapshot stays as it is. */
  dispose(): void {
    this.unsubscribe();
    this.listeners.clear();
    this.open = false;
    this.cancelRetry();
  }

  /** Fetch the project anew. A fetch asked for while one is under way is made after it. */
  refetch(): Promise<void> {
    if (this.fetching) {
      this.fetchAgain = true;
      return this.fetching;
    }
    this.cancelRetry();
    this.held = [];
    const epoch = this.epoch;
    this.fetching = (async () => {
      try {
        const got = await callEngine(this.bridge, 'project.get');
        if (epoch !== this.epoch) return;
        let revision = got.revision;
        let project: Project = { ...got.project, next_id: got.next_id };
        for (const change of this.held) {
          if (change.revision <= revision) continue;
          if ((change.base ?? change.revision - 1) !== revision) {
            this.fetchAgain = true;
            break;
          }
          try {
            project = applyChange(project, change);
          } catch {
            this.fetchAgain = true;
            break;
          }
          revision = change.revision;
        }
        this.failures = 0;
        this.set({ status: 'connected', revision, project, error: null });
      } catch (error) {
        if (epoch !== this.epoch) return;
        this.set({ status: 'disconnected', error: error instanceof Error ? error.message : String(error) });
        // The engine is open, and answers again in a moment: a project that is long to read, or a
        // connection that is made again.
        if (this.open && !this.fetchAgain) {
          const wait = Math.min(RETRY_MOST_MS, RETRY_FIRST_MS * 2 ** this.failures);
          this.failures += 1;
          this.retry = setTimeout(() => {
            this.retry = null;
            if (epoch === this.epoch) void this.refetch();
          }, wait);
        }
      } finally {
        this.fetching = null;
        this.held = [];
        if (this.fetchAgain) {
          this.fetchAgain = false;
          void this.refetch();
        }
      }
    })();
    return this.fetching;
  }

  private cancelRetry(): void {
    if (this.retry !== null) clearTimeout(this.retry);
    this.retry = null;
  }

  private set(changed: Partial<StudioSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...changed };
    for (const listener of [...this.listeners]) listener();
  }

  private heard(event: EngineEvent): void {
    // What an engine that was open before the one that is told of last is passed over. A session
    // that comes after it is heard: one that is being opened, or could not be.
    const { session } = this.snapshot;
    if (typeof event.session === 'number' && session !== null && event.session < session) return;
    if (event.kind !== 'opened' && event.kind !== 'taking-over' && event.kind !== 'error'
      && typeof event.session === 'number' && session !== null && event.session !== session) return;
    switch (event.kind) {
      case 'opened':
        this.epoch += 1;
        this.open = true;
        this.failures = 0;
        if (event.session !== session) {
          // Another engine, maybe of another project: nothing of the one before is shown of it.
          this.set({
            session: event.session, status: 'disconnected', revision: null, project: null, transports: { source: null, timeline: null }, proxies: {},
            views: { source: { state: 'none' }, timeline: { state: 'none' } }, error: null, takingOver: false,
          });
        } else if (this.snapshot.takingOver) this.set({ takingOver: false });
        void this.refetch();
        return;
      case 'exited':
        this.epoch += 1;
        this.open = false;
        this.fetchAgain = false;
        this.cancelRetry();
        this.set({ status: 'disconnected', transports: { source: null, timeline: null }, views: { source: { state: 'none' }, timeline: { state: 'none' } } });
        return;
      case 'taking-over':
        this.set({ takingOver: true });
        return;
      case 'error':
        // An engine that could not be opened is taken over no more.
        this.set({ error: event.message, takingOver: false });
        return;
      case 'attached':
        this.set({ views: { ...this.snapshot.views, [event.monitor]: { state: 'attached', adapter: event.adapter, backend: event.backend } } });
        return;
      case 'unsupported':
        this.set({ views: { ...this.snapshot.views, [event.monitor]: { state: 'unsupported', reason: event.reason } } });
        return;
      case 'notification':
        this.told(event.method, event.params);
        return;
      default:
    }
  }

  private told(method: string, params: unknown): void {
    if (params === null || typeof params !== 'object') return;
    if (method === 'project.changed') this.changed(params as ProjectChanged);
    else if (method === 'transport.state') {
      const state = params as TransportState;
      if (state.monitor === 'source' || state.monitor === 'timeline') {
        this.set({ transports: { ...this.snapshot.transports, [state.monitor]: state } });
      }
    }
    else if (method === 'media.proxy') {
      const told = params as ProxyEvent;
      this.set({ proxies: { ...this.snapshot.proxies, [told.media]: told } });
    } else if (method === 'engine.error') this.set({ error: (params as EngineErrorNotice).message });
  }

  private changed(change: ProjectChanged): void {
    if (this.fetching) {
      this.held.push(change);
      return;
    }
    const { status, revision, project } = this.snapshot;
    // Before the copy was fetched, or once its engine ended, a fetch to come takes it in.
    if (status !== 'connected' || revision === null || project === null) return;
    if (change.revision <= revision) return;
    // A change applies to the revision before it, or to its `base` (a gesture's, told together).
    if ((change.base ?? change.revision - 1) !== revision) {
      void this.refetch();
      return;
    }
    let at = 0;
    try {
      let next: Project = { ...project, next_id: change.next_id };
      for (const op of change.done) { next = applyOp(next, op); at++; }
      this.set({ revision: change.revision, project: next });
    } catch (error) {
      // The copy and the engine disagree (or the page does not know an operation): it is fetched
      // again whole. This should not happen, so it is said, with the operation that failed.
      console.warn(`The copy of the project could not apply ${change.done[at]?.op ?? 'a change'} (operation ${at + 1} of revision ${change.revision}); it is fetched again.`, error);
      void this.refetch();
    }
  }
}
