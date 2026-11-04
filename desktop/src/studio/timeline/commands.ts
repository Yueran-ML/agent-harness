import type { Alignment, CrossfadeLevel, Cue, InsertsOf, KeyInterp, Op, Speed, Time, TitleStyle, TrackKind, TransitionKind } from '../engine';

/** Input commands, not project operations: the engine expands them into the accepted `done` list. */
export type Material =
  | { kind: 'media'; media: string; from?: Time; to?: Time; length?: number }
  | { kind: 'title'; text: string; style: TitleStyle; length: number }
  | { kind: 'color'; color: string; length: number };
export interface Targets { video_track?: string | null; audio_track?: string | null }
export type Edit = { timeline: string } & (
  | ({ edit: 'insert' | 'overwrite' | 'replace' | 'smart_insert' | 'ripple_overwrite'; source: Material; at: number } & Targets)
  | ({ edit: 'append'; source: Material } & Targets)
  | { edit: 'place_on_top'; source: Material; at: number }
  | { edit: 'lift' | 'ripple_delete'; clips: string[]; linked: boolean }
  | { edit: 'ripple_delete_range'; from: number; to: number; tracks?: string[] }
  /**
   * `straddling: 'stay'` leaves clips of other tracks across the gap where they are (as Track 1 of
   * the Cut page closes its gaps), instead of refusing; what starts after the gap still moves back.
   */
  | { edit: 'close_gap'; track: string; at: number; straddling?: 'refuse' | 'stay' }
  | { edit: 'blade'; at: number; tracks?: string[]; linked: boolean }
  | { edit: 'move'; clips: string[]; delta: number; to_track?: string; overwrite: boolean; linked: boolean }
  | { edit: 'insert_move'; clips: string[]; at: number; linked: boolean }
  | { edit: 'trim'; clip: string; edge: 'in' | 'out'; delta: number; ripple: boolean; linked: boolean }
  | { edit: 'roll'; left: string; right: string; delta: number; linked: boolean }
  | { edit: 'slip' | 'slide'; clip: string; delta: number; linked: boolean }
  /** `linked`: a transition of picture brings a crossfade at the cut of the sound linked to its clips (p. 973). */
  | { edit: 'add_transition'; left: string; right: string; kind?: TransitionKind; length: number; align?: Alignment; linked?: boolean }
  | { edit: 'remove_transition'; id: string }
  | { edit: 'set_transition'; id: string; kind?: TransitionKind; length?: number; align?: Alignment; fade_in_level_db?: CrossfadeLevel; fade_out_level_db?: CrossfadeLevel }
  | { edit: 'set_speed'; clip: string; speed: Speed; reverse?: boolean; ripple: boolean; linked: boolean }
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
);
/** An attribute of a clip that Paste/Remove Attributes takes (p. 3842). */
export type ClipAttribute = 'volume' | 'plugins' | 'equalizer';
export type Change = Op | Edit;
export interface Transaction { ops: Change[]; revision?: number }
