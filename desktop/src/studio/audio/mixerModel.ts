import type { AutomationArm, Bus, ClipTrack, Lane, Lanes, LoudnessStandard, Strip, Timeline, Track } from '../engine';
import type { Change } from '../timeline/commands';
import { clamp, FADER_GAIN } from './taper';

export interface Modifiers { altKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean }
export interface MixerStrip {
  key: string; id: string; kind: 'track' | 'bus'; name: string; color: string; number: number; channels: number;
  db: number; mute: boolean; solo: boolean; safe: boolean; locked: boolean; enabled: boolean;
  strip?: Strip; lanes?: Lanes; arm?: AutomationArm; outputs: string[];
}
export const formatDb = (value: number) => value <= -100 ? '−∞' : value.toFixed(1);
export function mixerStrips(timeline: Timeline): MixerStrip[] {
  const buses = timeline.buses ?? [];
  const make = (owner: Track | Bus, kind: 'track' | 'bus', number: number): MixerStrip => {
    const track = kind === 'track' ? owner as ClipTrack : null;
    const sound = owner as Bus;
    const format = track?.format ?? (kind === 'bus' ? (owner as Bus).format : 'stereo');
    const channels = format === 'mono' ? 1 : format === 'stereo' ? 2 : format === 'adaptive' ? track?.channels ?? 2 : format.split('.').reduce((sum, n) => sum + Number(n), 0);
    // RawTrack -> place_outputs resolves only B1, never an arbitrary first bus.
    const outputs = sound.strip?.outputs ?? (!sound.strip && kind === 'track' && buses.some(b => b.id === 'B1') ? [{ bus: 'B1', on: true }] : []);
    return { key: `${kind}:${owner.id}`, id: owner.id, kind, name: owner.name, color: owner.color ?? '#5aa0e0ff', number, channels,
      db: sound.strip?.fader_db ?? 0, mute: sound.mute ?? false, solo: sound.solo ?? false, safe: sound.solo_safe ?? false,
      enabled: track?.enabled ?? true, locked: track?.locked ?? false, strip: sound.strip, lanes: sound.lanes, arm: sound.automation_arm,
      outputs: outputs.filter(o => o.on).map(o => buses.find(b => b.id === o.bus)?.name ?? o.bus) };
  };
  return [...timeline.tracks.filter(t => t.kind === 'audio').map((track, i) => make(track, 'track', i + 1)), ...buses.map((bus, i) => make(bus, 'bus', i + 1))];
}
/** model/audio/param.rs Lane::value_at: linear dB, switches hold, endpoints hold; no taper interpolation. */
export function laneValue(lane: Lane, seconds: number): number {
  const points = lane.points;
  if (!points.length) return 0;
  const at = seconds * points[0].at.scale;
  if (at < points[0].at.ticks) return points[0].value;
  let i = 0; while (i + 1 < points.length && points[i + 1].at.ticks <= at) i++;
  const from = points[i], to = points[i + 1];
  if (!to || lane.kind === 'switch') return from.value;
  const share = (at - from.at.ticks) / (to.at.ticks - from.at.ticks);
  return lane.scale === 'log' ? Math.exp(Math.log(from.value) + (Math.log(to.value) - Math.log(from.value)) * share) : from.value + (to.value - from.value) * share;
}
export function faderReading(strip: MixerStrip, seconds: number, playback = true) {
  const lane = strip.lanes?.fader_db, hasLane = Boolean(lane);
  const automated = playback && hasLane && strip.arm !== 'off';
  return { value: automated ? laneValue(lane!, seconds) : strip.db, automated, hasLane };
}
export function stripChange(timeline: string, strip: MixerStrip, patch: Record<string, unknown>, follow = false): Change {
  return strip.kind === 'track' ? { edit: 'set_track_audio', timeline, track: strip.id, patch, follow_groups: follow }
    : { edit: 'set_bus', timeline, id: strip.id, patch };
}
export function groupMembers(timeline: Timeline, strips: MixerStrip[], seeds: MixerStrip[], control: 'fader' | 'mute' | 'solo', clutch: boolean): MixerStrip[] {
  const ids = new Set(seeds.map(s => s.key));
  if (!clutch) for (const seed of seeds.filter(s => s.kind === 'track')) for (const group of timeline.track_groups ?? []) {
    if (group.active === false || (group.linked && !group.linked.includes(control)) || !group.members.includes(seed.id)) continue;
    for (const id of group.members) ids.add(`track:${id}`);
  }
  return strips.filter(s => ids.has(s.key) && !s.locked);
}
export function operationTargets(strips: MixerStrip[], key: string, selected: readonly string[], modifiers: Modifiers): MixerStrip[] {
  return strips.filter(s => !s.locked && (modifiers.altKey ? modifiers.ctrlKey || selected.includes(s.key) || s.key === key : s.key === key));
}
export interface FaderPlan { source: MixerStrip; members: MixerStrip[]; follow: boolean; restore: Change[] }
export function faderPlan(timeline: Timeline, key: string, selected: readonly string[], modifiers: Modifiers, playback = true): FaderPlan {
  const strips = mixerStrips(timeline), source = strips.find(s => s.key === key)!;
  const seeds = operationTargets(strips, key, selected, modifiers).filter(s => !faderReading(s, 0, playback).automated);
  const clutch = Boolean(modifiers.ctrlKey && modifiers.shiftKey);
  const linked = groupMembers(timeline, strips, seeds, 'fader', clutch);
  const members = linked.filter(s => !faderReading(s, 0, playback).automated);
  // The engine's ordinary group follower edit has no lane-readonly filter. When a group
  // includes one of those strips, use the already deduplicated writable members explicitly.
  return { source, members, follow: members.some(s => s.key === source.key) && linked.length === members.length && !modifiers.altKey && !clutch && source.kind === 'track',
    restore: members.map(s => stripChange(timeline.id, s, { strip: { fader_db: s.db } })) };
}
export function faderChanges(timeline: string, plan: FaderPlan, delta: number): Change[] {
  const value = (s: MixerStrip) => clamp(Math.max(-100, s.db) + delta, ...FADER_GAIN);
  if (plan.follow) {
    // Restart the captured group before each absolute preview: a follower that clipped at an
    // endpoint must recover its original offset when the pointer returns. The engine applies
    // this transaction atomically and expands the ordinary follow_groups edit itself.
    const beyond = Math.max(-100, plan.source.db) + delta !== value(plan.source);
    return [...(plan.members.length > 1 ? plan.restore : []), stripChange(timeline, plan.source, { strip: { fader_db: value(plan.source) } }, true),
      // The engine sees only the clamped source. Continue the requested group delta past that
      // stop for the other members, using absolute values once each (C-F4).
      ...(beyond ? plan.members.filter(s => s.key !== plan.source.key).map(s => stripChange(timeline, s, { strip: { fader_db: value(s) } })) : [])];
  }
  return plan.members.map(s => stripChange(timeline, s, { strip: { fader_db: value(s) } }));
}
/** server/meters.rs::standard_of: settings enums and analysis standard IDs are different. */
export function analysisStandardId(standard: string): string {
  const ids: Record<LoudnessStandard, string> = { bs1770_1: 'bs1770-1', bs1770_4: 'bs1770-4', atsc_a85: 'atsc-a85', ebu_r128: 'ebu-r128',
    op_59: 'op-59', tr_b32: 'tr-b32', agcom_219: 'agcom-219', netflix: 'netflix', youtube: 'youtube', disney_2_0: 'disney-2.0', disney_5_1: 'disney-5.1',
    apple_podcasts: 'apple-podcasts', spotify: 'spotify', aes_streaming: 'aes-streaming-dialogue', web_video: 'web-video' };
  return ids[standard as LoudnessStandard] ?? standard;
}
