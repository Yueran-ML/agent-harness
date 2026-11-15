import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useOverlay } from '../../app/overlay';
import { TimelineMenu } from './SubtitleTrack';
import type { AudioKey, AudioParam, Clip, KeyInterp, Rate } from '../engine';
import { CLIP_GAIN, clamp, dbToPosition, positionToDb } from '../audio/taper';
import { audioValue, frameAtSource, keyTime, seconds, sourceAtFrame, sourceSampleBounds, sourceSampleTime, type CopiedAudioKey } from './audioKeys';
import type { AudioGesture } from './useAudioGesture';
import type { Change } from './commands';
import type { Marks } from './snap';
import { backwardAt } from '../retime';
import { useFairlightText } from '../fairlight/words';

export interface AudioClipboard { keys: CopiedAudioKey[] }
interface Props {
  clip: Clip; timeline: string; rate: Rate; scale: number; height: number; sampleRate: number; snapping: boolean;
  frame: number; session: number; from: number; to: number; marks: Marks; locked: boolean; gesture: AudioGesture; clipboard: AudioClipboard;
  onSeek: (frame: number, scrubbing: boolean) => void; children?: ReactNode;
}
interface Drag {
  x: number; y: number; height: number; width: number; initial: AudioParam; value: number;
  indices: number[] | null; key: number | null; axis: 'x' | 'y' | null; moved: boolean; range: [number, number] | null;
  /** Plain press keeps the group for dragging; a release without movement selects only this key. */
  click: number | null;
}
const keyId = (key: AudioKey) => seconds(key.at);
const mirrored = (interp: KeyInterp): KeyInterp => interp === 'ease_in' ? 'ease_out' : interp === 'ease_out' ? 'ease_in' : interp;
export function VolumeLine({ clip, timeline, rate, scale, height, sampleRate, snapping, frame, session, from, to, marks, locked, gesture, clipboard, onSeek, children }: Props) {
  const t = useFairlightText();
  const group = useRef<SVGGElement>(null);
  const drag = useRef<Drag | null>(null);
  const [preview, setPreview] = useState<AudioParam | null>(null);
  const [selected, setSelected] = useState<number[]>([]);
  const anchor = useRef<number | null>(null);
  // Audio keys can share a displayed frame. Keep their exact source identity between bracket presses.
  const navigation = useRef<{ at: number; displayed: number } | null>(null);
  useEffect(() => { if (navigation.current && frame !== navigation.current.displayed) navigation.current = null; }, [frame]);
  useEffect(() => { navigation.current = null; }, [clip, timeline, session]);
  const [menu, setMenu] = useState<{ x: number; y: number; at: number } | null>(null);
  useOverlay(menu !== null);
  const closeMenu = useCallback(() => { setMenu(null); group.current?.focus(); }, []);
  const [tip, setTip] = useState<{ value: number; delta: number } | null>(null);
  useEffect(() => () => gesture.cancel(), []);
  const active = gesture.active();
  const param = active && preview !== null ? preview : clip.gain_db;
  const keys = typeof param === 'number' ? [] : param.keys;
  const width = Math.max(1, clip.length * scale);
  const xOf = (at: number) => (frameAtSource(clip, rate, at) - clip.start) * scale;
  const yOf = (value: number) => 12 + 76 * (1 - dbToPosition(value, CLIP_GAIN));
  const setGain = (gain_db: AudioParam): Change => ({ op: 'set_clip', timeline, id: clip.id, gain_db });
  const geometry = (event: PointerEvent<SVGElement>) => {
    const box = event.currentTarget.ownerSVGElement!.getBoundingClientRect();
    return { x: (event.clientX - box.left) * width / (box.width || width), y: (event.clientY - box.top) * 100 / (box.height || 64), box };
  };
  const choose = (index: number, event: { ctrlKey: boolean; shiftKey: boolean }) => {
    const at = keyId(keys[index]);
    let next: number[];
    if (event.shiftKey && anchor.current !== null) next = keys.filter(key => keyId(key) >= Math.min(anchor.current!, at) && keyId(key) <= Math.max(anchor.current!, at)).map(keyId);
    else if (event.ctrlKey) next = selected.includes(at) ? selected.filter(value => value !== at) : [...selected, at];
    else next = selected.includes(at) ? selected : [at];
    if (!event.shiftKey) anchor.current = at;
    setSelected(next); return next;
  };
  const down = (event: PointerEvent<SVGGElement>, index: number | null, segment: number[] | null) => {
    if (event.button !== 0) return;
    event.stopPropagation(); event.preventDefault(); group.current?.focus(); setMenu(null); navigation.current = null;
    if (locked) return;
    const point = geometry(event);
    if (event.altKey) {
      if (index !== null && event.ctrlKey) {
        gesture.commit([{ edit: 'clip_gain_keys', timeline, clip: clip.id, action: 'remove', at: keys[index].at }]);
        setSelected(selected.filter(value => value !== keyId(keys[index])));
      } else if (!event.ctrlKey) {
        const at = keyTime(clip, rate, clip.start + point.x / scale, sampleRate, snapping);
        gesture.commit([{ edit: 'clip_gain_keys', timeline, clip: clip.id, action: 'add', at, value: audioValue(param, seconds(at)) }]);
        setSelected([seconds(at)]); anchor.current = seconds(at);
      }
      return;
    }
    const selection = index === null ? [] : choose(index, event);
    if (index === null) setSelected([]);
    const initial = clip.gain_db;
    if (!gesture.start([setGain(initial)])) return;
    const indices = index === null ? segment : keys.map((key, i) => selection.includes(keyId(key)) ? i : -1).filter(i => i >= 0);
    const at = sourceAtFrame(clip, rate, clip.start + point.x / scale);
    let range: [number, number] | null = null;
    if (index === null && marks.in !== null && marks.out !== null) {
      const from = Math.max(clip.start, marks.in), to = Math.min(clip.start + clip.length, marks.out);
      if (to > from && clip.start + point.x / scale >= from && clip.start + point.x / scale <= to) {
        const a = sourceAtFrame(clip, rate, from), b = sourceAtFrame(clip, rate, to); range = [Math.min(a, b), Math.max(a, b)];
      }
    }
    drag.current = { x: event.clientX, y: event.clientY, height: point.box.height || 64, width: point.box.width || width,
      initial, value: index === null ? audioValue(initial, at) : keys[index].value, indices, key: index, axis: null, moved: false, range,
      click: index !== null && !event.ctrlKey && !event.shiftKey ? keyId(keys[index]) : null };
    group.current?.setPointerCapture?.(event.pointerId);
  };
  const move = (event: PointerEvent<SVGGElement>) => {
    const d = drag.current;
    if (!d) return;
    event.stopPropagation();
    if (!gesture.active()) { drag.current = null; setPreview(null); setTip(null); return; }
    let dx = event.clientX - d.x, dy = event.clientY - d.y;
    if (!d.moved && Math.hypot(dx, dy) < 3) return;
    if (d.key !== null && event.shiftKey && d.axis === null) d.axis = Math.abs(dx) > Math.abs(dy) ? 'x' : 'y';
    if (d.axis === 'x') dy = 0; else if (d.axis === 'y') dx = 0;
    d.moved = true;
    const fine = d.key === null && event.shiftKey ? .1 : 1;
    const value = dy === 0 ? d.value : positionToDb(dbToPosition(d.value, CLIP_GAIN) - dy / (d.height * .76) * fine, CLIP_GAIN);
    const delta = value - d.value;
    let next: AudioParam;
    if (typeof d.initial === 'number') next = value;
    else {
      const list = d.initial.keys;
      let shift = 0;
      if (d.key !== null && d.indices?.length) {
        const original = list[d.key];
        const desired = frameAtSource(clip, rate, seconds(original.at)) + dx * width / d.width / scale;
        shift = seconds(keyTime(clip, rate, desired, sampleRate, snapping)) - seconds(original.at);
        const ends = sourceSampleBounds(clip, rate, sampleRate).map(ticks => ticks / sampleRate);
        let low = -Infinity, high = Infinity;
        for (const i of d.indices) {
          const at = seconds(list[i].at);
          low = Math.max(low, ends[0] - at); high = Math.min(high, ends[1] - at);
          if (i > 0 && !d.indices.includes(i - 1)) low = Math.max(low, seconds(list[i - 1].at) + 1 / sampleRate - at);
          if (i + 1 < list.length && !d.indices.includes(i + 1)) high = Math.min(high, seconds(list[i + 1].at) - 1 / sampleRate - at);
        }
        shift = clamp(shift, low, high);
      }
      next = { keys: list.map((key, i) => d.indices?.includes(i) ? { ...key,
        at: d.key === null ? key.at : sourceSampleTime(clip, rate, seconds(key.at) + shift, sampleRate), value: clamp(key.value + delta, ...CLIP_GAIN) } : key) };
      if (d.key !== null) setSelected(next.keys.filter((_, i) => d.indices?.includes(i)).map(keyId));
    }
    // Reset to the captured curve before each range edit: total pointer delta is never accumulated.
    const ops: Change[] = d.range ? [setGain(d.initial), { edit: 'clip_gain_range', timeline, clip: clip.id,
      from: sourceSampleTime(clip, rate, d.range[0], sampleRate), to: sourceSampleTime(clip, rate, d.range[1], sampleRate), delta_db: delta }] : [setGain(next)];
    if (d.range && delta === 0) ops.splice(1);
    setPreview(d.range ? null : next); setTip({ value, delta }); gesture.preview(ops);
  };
  const finish = (cancel: boolean) => {
    const clicked = drag.current;
    if (!cancel && clicked && !clicked.moved && clicked.click !== null) setSelected([clicked.click]);
    drag.current = null; setPreview(null); setTip(null);
    if (cancel) gesture.cancel(); else gesture.end();
  };
  const keyDown = (event: KeyboardEvent<SVGGElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.target instanceof Element && event.target.closest('.audio-fades')) return;
    const key = event.key.toLowerCase();
    const ctrl = event.ctrlKey;
    const handled = key === 'escape' || key === '[' || key === ']' || key === 'backspace' || key === 'delete' || (ctrl && ['c', 'x', 'v'].includes(key));
    if (!handled) return;
    event.preventDefault(); event.stopPropagation();
    if (key === 'escape') { finish(true); setMenu(null); return; }
    if (key === '[' || key === ']') {
      // A key of a time that a clip with speed points never comes by is nowhere on the clip to go to.
      const ordered = keys.map(each => ({ at: keyId(each), frame: frameAtSource(clip, rate, keyId(each)) }))
        .filter(each => !clip.retime || (each.frame >= clip.start && each.frame <= clip.start + clip.length)).sort((a, b) => a.frame - b.frame);
      const index = navigation.current ? ordered.findIndex(each => each.at === navigation.current!.at) : -1;
      const target = index >= 0 ? ordered[index + (key === '[' ? -1 : 1)]
        : key === '[' ? ordered.filter(each => each.frame < frame - 1e-8).at(-1) : ordered.find(each => each.frame > frame + 1e-8);
      if (target) {
        const displayed = Math.round(target.frame);
        navigation.current = { at: target.at, displayed };
        setSelected([target.at]); anchor.current = target.at;
        onSeek(displayed, false);
      }
      return;
    }
    const chosen = keys.filter(each => selected.includes(keyId(each)));
    if (ctrl && (key === 'c' || key === 'x') && chosen.length) {
      const ordered = chosen.map(each => ({ ...each, frame: frameAtSource(clip, rate, seconds(each.at)) })).sort((a, b) => a.frame - b.frame);
      // Where the clip plays backward the keys are passed the other way: with speed points, where the segment that a key is in does.
      clipboard.keys = ordered.map(each => ({ offset: (each.frame - ordered[0].frame) * rate.den / rate.num, value: each.value,
        interp: backwardAt(clip, each.frame - clip.start) ? mirrored(each.interp ?? 'linear') : each.interp }));
    }
    if (locked || gesture.active()) return;
    if (key === 'backspace' || key === 'delete' || (ctrl && key === 'x')) {
      gesture.commit(chosen.map(each => ({ edit: 'clip_gain_keys', timeline, clip: clip.id, action: 'remove', at: each.at }))); setSelected([]);
    } else if (ctrl && key === 'v' && clipboard.keys.length) {
      const pasted = clipboard.keys.map(each => ({ ...each, frame: frame + each.offset * rate.num / rate.den }));
      if (pasted.some(each => each.frame < clip.start || each.frame > clip.start + clip.length)) return;
      const additions = pasted.map(each => ({ edit: 'clip_gain_keys' as const, timeline, clip: clip.id, action: 'add' as const,
        at: keyTime(clip, rate, each.frame, sampleRate, false), value: each.value,
        interp: backwardAt(clip, each.frame - clip.start) ? mirrored(each.interp ?? 'linear') : each.interp }));
      gesture.commit(additions); setSelected(additions.map(each => seconds(each.at)));
    }
  };
  const segments: { from: number; to: number; indices: number[] | null }[] = [];
  const ordered = keys.map((key, index) => ({ x: xOf(seconds(key.at)), index })).sort((a, b) => a.x - b.x);
  const left = Math.max(0, (from - clip.start) * scale), right = Math.min(width, (to - clip.start) * scale);
  const points = [left, ...ordered.map(each => each.x).filter(x => x > left && x < right), right];
  for (let i = 0; i < points.length - 1; i++) {
    const middle = (points[i] + points[i + 1]) / 2;
    const before = ordered.filter(each => each.x <= middle).at(-1), after = ordered.find(each => each.x >= middle);
    segments.push({ from: points[i], to: points[i + 1], indices: keys.length ? [...new Set([before?.index, after?.index].filter((n): n is number => n !== undefined))] : null });
  }
  return <g ref={group} tabIndex={0} role="group" aria-label={t('Clip gain')} className="audio-volume" data-testid={`volume-line-${clip.id}`}
    onBlur={event => { if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) navigation.current = null; }}
    onKeyDown={keyDown} onPointerMove={move} onPointerUp={event => { if (drag.current) { event.stopPropagation(); finish(false); } }}
    onPointerCancel={event => { event.stopPropagation(); finish(true); }} onLostPointerCapture={() => { if (drag.current) finish(true); }}
    onPointerDown={event => { if (event.target === event.currentTarget) down(event, null, null); }}>
    {segments.map((segment, i) => {
      const count = Math.max(1, Math.ceil((segment.to - segment.from) / 8));
      const path = Array.from({ length: count + 1 }, (_, n) => { const x = segment.from + (segment.to - segment.from) * n / count;
        return `${n === 0 ? 'M' : 'L'}${x},${yOf(audioValue(param, sourceAtFrame(clip, rate, clip.start + x / scale)))}`; }).join(' ');
      return <g key={i} onPointerDown={event => down(event, null, segment.indices)} onDoubleClick={event => { event.stopPropagation(); if (!locked) gesture.commit([setGain(0)]); }}>
        <path d={path} className="audio-gain-visible" /><path d={path} className="audio-gain-hit" />
      </g>;
    })}
    {children}
    {keys.map((key, index) => { const x = xOf(seconds(key.at)); return x < left || x > right ? null : <g key={index} transform={`translate(${x},${yOf(key.value)}) scale(1,${100 / height})`}>
      <circle r={4} className={`audio-gain-dot${selected.includes(keyId(key)) ? ' selected' : ''}`} />
      <circle r={6} className="audio-gain-key" data-audio-key={index}
      data-selected={selected.includes(keyId(key))} onPointerDown={event => down(event, index, null)}
      onContextMenu={event => { event.preventDefault(); event.stopPropagation(); navigation.current = null; if (!selected.includes(keyId(key))) setSelected([keyId(key)]); setMenu({ x: event.clientX, y: event.clientY, at: keyId(key) }); }} />
    </g>; })}
    {tip && active && <text className="audio-gain-tip" x={left + 8} y={11 * 100 / height} style={{ fontSize: 10 * 100 / height }}>{tip.value.toFixed(1)} dB ({tip.delta >= 0 ? '+' : ''}{tip.delta.toFixed(1)} dB)</text>}
    {menu && createPortal(<div className="audio-key-popup" onKeyDown={event => event.stopPropagation()}><TimelineMenu label={t('Keyframe interpolation')} onClose={closeMenu} menu={{
      x: Math.min(menu.x, Math.max(0, window.innerWidth - 190)), y: Math.min(menu.y, Math.max(0, window.innerHeight - 120)),
      items: (['linear', 'ease_in', 'ease_out'] as const).map(interp => ({ id: interp, disabled: locked,
        label: t(interp === 'linear' ? 'Linear' : interp === 'ease_in' ? 'Ease In' : 'Ease Out'),
        run: () => gesture.commit(keys.filter(key => selected.includes(keyId(key)) || keyId(key) === menu.at)
          .map(key => ({ edit: 'clip_gain_keys', timeline, clip: clip.id, action: 'set_interp', at: key.at, interp }))),
      })),
    }} /></div>, document.body)}
  </g>;
}
