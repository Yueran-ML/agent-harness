import type { Alignment, Cue, Op, Speed, Time, TitleStyle, TrackKind, TransitionKind } from '../engine';

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
  | { edit: 'add_transition'; left: string; right: string; kind?: TransitionKind; length: number; align?: Alignment }
  | { edit: 'remove_transition'; id: string }
  | { edit: 'set_transition'; id: string; kind?: TransitionKind; length?: number; align?: Alignment }
  | { edit: 'set_speed'; clip: string; speed: Speed; reverse?: boolean; ripple: boolean; linked: boolean }
  | { edit: 'add_cue'; track: string; start: number; length: number; text: string }
  | { edit: 'set_cue'; id: string; start?: number; length?: number; text?: string }
  | { edit: 'remove_cue'; id: string }
  | { edit: 'cues_from'; track: string; items: Omit<Cue, 'id'>[] }
  | { edit: 'add_track'; kind: TrackKind; index?: number; name?: string; language?: string }
  | { edit: 'remove_track'; track: string }
);
export type Change = Op | Edit;
export interface Transaction { ops: Change[]; revision?: number }
