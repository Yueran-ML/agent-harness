import type { Alignment, CrossfadeLevel, Cue, Insert, InsertsOf, KeyInterp, LanePoint, LanesOf, Op, Speed, Time, TitleStyle, TrackKind, TransitionKind } from '../engine';
import type { ClipClipboard } from './clipboard';

/** Input commands, not project operations: the engine expands them into the accepted `done` list. */
export type Material =
  | { kind: 'media'; media: string; from?: Time; to?: Time; length?: number }
  | { kind: 'title'; text: string; style: TitleStyle; length: number }
  | { kind: 'color'; color: string; length: number };
export interface Targets { video_track?: string | null; audio_track?: string | null }
export type Edit = { timeline: string } & (
  | { edit: 'cut_clips'; clips: string[]; from?: number; to?: number; linked: boolean; grouped: boolean }
  | { edit: 'paste_clips'; clipboard: ClipClipboard; at: number; tracks: Record<string, string> }
  | ({ edit: 'insert' | 'overwrite' | 'replace' | 'smart_insert' | 'ripple_overwrite'; source: Material; at: number } & Targets)
  | ({ edit: 'append'; source: Material } & Targets)
  | { edit: 'place_on_top'; source: Material; at: number }
  /**
   * `grouped: true` (an editor's own action; left out by edits that a client puts together, such
   * as clearing where copies go) makes the active groups of tracks that link Editing take the
   * frames of the clips (or, for `blade` and `ripple_delete_range`, the tracks given) along on
   * their other tracks; see PROTOCOL, Groups of tracks that link Editing.
   */
  | { edit: 'lift' | 'ripple_delete'; clips: string[]; linked: boolean; grouped?: boolean }
  | { edit: 'ripple_delete_range'; from: number; to: number; tracks?: string[]; grouped?: boolean }
  /**
   * `straddling: 'stay'` leaves clips of other tracks across the gap where they are (as Track 1 of
   * the Cut page closes its gaps), instead of refusing; what starts after the gap still moves back.
   */
  | { edit: 'close_gap'; track: string; at: number; straddling?: 'refuse' | 'stay' }
  | { edit: 'blade'; at: number; tracks?: string[]; linked: boolean; grouped?: boolean }
  | { edit: 'move'; clips: string[]; delta: number; to_track?: string; overwrite: boolean; linked: boolean; grouped?: boolean }
  | { edit: 'insert_move'; clips: string[]; at: number; linked: boolean; grouped?: boolean }
  /**
   * Copies of clips, each from `start` on `track` (an Alt-drag duplicate or a paste): what is there
   * is cleared as an overwrite clears it, and with Automation Follows Edit each copy takes a copy
   * of the lanes over its clip. The desktop still sends its own clearing and `insert_clip`s (T19).
   */
  | { edit: 'place_copies'; grouped?: boolean; copies: { clip: string; track: string; start: number }[] }
  | { edit: 'trim'; clip: string; edge: 'in' | 'out'; delta: number; ripple: boolean; linked: boolean }
  | { edit: 'roll'; left: string; right: string; delta: number; linked: boolean }
  | { edit: 'slip' | 'slide'; clip: string; delta: number; linked: boolean }
  /** `linked`: a transition of picture brings a crossfade at the cut of the sound linked to its clips (p. 973). */
  | { edit: 'add_transition'; left: string; right: string; kind?: TransitionKind; length: number; align?: Alignment; linked?: boolean }
  | { edit: 'remove_transition'; id: string }
  | { edit: 'set_transition'; id: string; kind?: TransitionKind; length?: number; align?: Alignment; fade_in_level_db?: CrossfadeLevel; fade_out_level_db?: CrossfadeLevel }
  | { edit: 'set_speed'; clip: string; speed: Speed; reverse?: boolean; ripple: boolean; linked: boolean }
  // The edits of the Retime Controls (PROTOCOL.md, Edits; Reference Manual 20, pp. 1070–1082). `at` is a
  // frame of the clip, counted from its first. With `linked`, the clips linked to it take the change at
  // the same frame of the timeline; with `ripple`, what follows moves with the end of the clip.
  /** A speed point at frame `at`: the segment there becomes two of its speed. */
  | { edit: 'add_speed_point'; clip: string; at: number; linked?: boolean }
  /** The segment that frame `at` is in plays at `speed`, the way that it went, and shows what it showed. */
  | { edit: 'set_segment_speed'; clip: string; at: number; speed: Speed; ripple?: boolean; linked?: boolean }
  /** The picture of frame `at` is held for `hold` frames more. */
  | { edit: 'freeze_frame'; clip: string; at: number; hold: number; ripple?: boolean; linked?: boolean }
  /** The segment that frame `at` is in plays the other way. */
  | { edit: 'reverse_segment'; clip: string; at: number; linked?: boolean }
  /** The speed point at the left of the segment that frame `at` is in is cleared. */
  | { edit: 'clear_speed_point'; clip: string; at: number; linked?: boolean }
  /** The clip plays at the speed of its source, forward, with no speed points. */
  | { edit: 'reset_speed'; clip: string; ripple?: boolean; linked?: boolean }
  | { edit: 'add_cue'; track: string; start: number; length: number; text: string }
  | { edit: 'set_cue'; id: string; start?: number; length?: number; text?: string }
  | { edit: 'remove_cue'; id: string }
  | { edit: 'cues_from'; track: string; items: Omit<Cue, 'id'>[] }
  | { edit: 'add_track'; kind: TrackKind; index?: number; name?: string; language?: string }
  | { edit: 'remove_track'; track: string }
  // The edits of sound (PROTOCOL.md, "Edits of sound").
  /** As `set_track_audio`; with `follow_groups` (true where left out), the active groups that the track is in follow it. */
  | { edit: 'set_track_audio'; track: string; patch: Record<string, unknown>; follow_groups?: boolean }
  /** Keys at times of the clip's media (`at`, `to`). */
  | { edit: 'clip_gain_keys'; clip: string; param?: 'gain_db' | 'pan'; action: 'add' | 'remove' | 'move' | 'set_interp'; at: Time; to?: Time; value?: number; interp?: KeyInterp }
  | { edit: 'clip_gain_range'; clip: string; from: Time; to: Time; delta_db: number }
  | { edit: 'nudge_clip_gain'; clips: string[]; delta_db: number; linked?: boolean }
  | { edit: 'paste_attributes'; from: string; to: string[]; items: ClipAttribute[] }
  | { edit: 'remove_attributes'; clips: string[]; items: ClipAttribute[] }
  | { edit: 'fade_to_playhead'; clips: string[]; side: 'in' | 'out'; at: number }
  /** As `set_bus`; a bus soloed while the solo is exclusive takes the solo off the others. */
  | { edit: 'set_bus'; id: string; patch: Record<string, unknown> }
  | { edit: 'copy_effects'; from: InsertsOf; to: InsertsOf[] }
  // The edits of automation (PROTOCOL.md, "Automation (T14, Ch. 178)"). Times are of the timeline.
  /** As the operation; the lanes of the insert that goes are taken away, and one put in its place is new. */
  | { edit: 'set_insert'; of: InsertsOf; slot: number; insert: Insert | null }
  /** Within `of` as the operation; to `to_of`, a new insert there, its lanes left behind and taken away. */
  | { edit: 'move_insert'; of: InsertsOf; from: number; to: number; to_of?: InsertsOf }
  | { edit: 'lane_points'; of: LanesOf; path: string; action: 'add' | 'remove' | 'move'; points?: { at: Time; value?: number }[]; at?: Time[]; by?: Time; delta?: number; grouped?: boolean }
  | { edit: 'lane_range'; of: LanesOf; path: string; from: Time; to: Time; action: 'delete' | 'cut' | 'paste' | 'shift_value' | 'thin'; points?: LanePoint[]; delta?: number; tolerance?: number; grouped?: boolean }
  /** The Pencil: the points drawn, from `from` to `to` in order, thinned by the engine. */
  | { edit: 'lane_draw'; of: LanesOf; path: string; from: Time; to: Time; points: LanePoint[]; tolerance?: number; grouped?: boolean }
  /** Fill Range, or Glide Range with `glide` (T16b), as `automation.fill` writes it. */
  | { edit: 'lane_fill'; lanes: (LanesOfLane & { value: number })[]; from: Time; to: Time; glide?: boolean }
  /** Paste of automation (T16b): what `automation.copy` gave, at `at` or over the range to `to`. */
  | { edit: 'automation_paste'; at: Time; to?: Time; lanes: CopiedLane[] }
  /** Erase of automation (T16b): the points of the range of the owners' lanes of the categories on. */
  | { edit: 'automation_erase'; of: LanesOf[]; from: Time; to: Time }
  /** Clear Track Automation: `then` keep (where left out), reset, or hold the value at `at`. */
  | { edit: 'clear_lanes'; of: LanesOf; paths?: string[]; then?: 'keep' | 'reset' | 'hold'; at?: Time }
  /** A pass of the recording of automation (T16), as the engine writes it at the stop or a Punch In. */
  | { edit: 'record_pass'; lanes: RecordedLane[] }
);
/** A lane of an owner (PROTOCOL.md, "Recording automation"). */
export interface LanesOfLane { of: LanesOf; path: string }
/**
 * A lane as `automation.copy` gives it and `automation_paste` puts it (T16b): the points of a range
 * at times from its start, or the value at a time (a snapshot).
 */
export interface CopiedLane extends LanesOfLane { points?: LanePoint[]; value?: number }
/** What a pass of recording wrote on one lane (PROTOCOL.md, "Recording automation"). */
export interface RecordedLane {
  of: LanesOf;
  path: string;
  /** The runs are offsets in dB of the lane as it was (Trim), not its values. */
  trim?: boolean;
  runs: { points: LanePoint[]; to: Time; then: 'return' | 'hold' | 'event' | 'cut'; glide?: Time }[];
}
/** An attribute of a clip that Paste/Remove Attributes takes (p. 3842). */
export type ClipAttribute = 'volume' | 'plugins' | 'equalizer';
export type Change = Op | Edit;
export interface Transaction { ops: Change[]; revision?: number }
