import { useContext, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type RefObject } from 'react';
import type { EngineBridge } from '../../bridge';
import {
  callEngine, type AutomationArm, type AutomationParameter, type Lane, type LanePoint, type Lanes, type LanesOf, type Timeline,
} from '../engine';
import { useAutomationParameters } from '../audio/useAutomationParameters';
import type { Change } from '../timeline/commands';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { HEADER_WIDTH } from '../timeline/geometry';
import { AudioPlayhead } from '../timeline/AudioOverlay';
import { snap, snapCandidates, type Marks } from '../timeline/snap';
import { editingAllowed } from '../keys';
import { keyOf, type Key } from '../../app/keys';
import {
  categories, categoryOfPath, laneValue, nudgeSamples, ownerKey, Pencil, pixelAtSample, positionValue, readNudge, sampleAtPixel,
  sampleTime, tickOf, valuePosition, type AutomationTarget,
} from './automation';
import { useAutomationTouch, sameTarget } from './AutomationTouch';
import { isLevelPath } from './automationPaste';
import { CoveringMenu } from '../color/CoveringMenu';
import type { Menu, MenuItem } from '../color/ColorMenu';
import { mixerStrips, operationTargets, stripChange, type Modifiers } from '../audio/mixerModel';
import { useFailureText, useFairlightText } from './words';
import { parameterLabel } from './parameterLabel';
import { PortalMenu } from '../audio/PortalMenu';
interface Props {
  bridge: Pick<EngineBridge, 'call'>;
  session: number;
  revision: number;
  timeline: Timeline;
  of: LanesOf;
  lanes?: Lanes;
  locked?: boolean;
  arm?: AutomationArm;
  scale: number;
  from: number;
  to: number;
  height: number;
  snapping: boolean;
  marks: () => Marks;
  path: string;
  pencil: boolean;
  register?: (owner: string, command: (key: Key) => boolean) => () => void;
  /** The keys of the selected tracks and buses (`track:A1`), which Alt+click on Arm arms too. */
  selection?: () => readonly string[];
  onActive?: () => void;
  onPath: (path: string) => void;
  onEdit: (ops: Change[]) => void;
  name: string;
}
/**
 * The keyboard clipboard of automation (X, C, V on a lane, p. 3898): one for every lane, so that
 * what is cut or copied on one lane is pasted on another. Its points are from 0, in samples at
 * `rate`; `parameter` is of the lane it was taken from.
 */
let laneClipboard: { points: LanePoint[]; rate: number; path: string; parameter: AutomationParameter } | null = null;

/**
 * The value of `source` at `value` as it is pasted on a lane of `path` (`parameter`): the same
 * value on a lane of the same setting, or from one level to another (a fader's and a send's, p.
 * 3914); else the same height on the lane.
 */
function pastedValue(value: number, source: NonNullable<typeof laneClipboard>, path: string, parameter: AutomationParameter) {
  if (source.path === path || (isLevelPath(source.path) && isLevelPath(path)))
    return value;
  return positionValue(valuePosition(value, source.parameter), parameter);
}

/**
 * The parameters of a lane as nested menus (p. 3896, 3904, 3909): the categories of recording,
 * then the groups that their names give (Pan ▸, Sends ▸ Bus 2 ▸, EQ ▸ Band 1 ▸, Dynamics ▸
 * Compressor ▸, Plugins ▸ Insert 1 ▸), a group of one taken into its parent.
 */
export function parameterMenu(
  parameters: readonly AutomationParameter[], label: (path: string) => string,
  category: (name: typeof categories[number]) => string,
  path: string, choose: (path: string) => void, none: string,
): MenuItem[] {
  interface Node { label: string; path?: string; children: Node[] }
  const items = (node: Node, id: string): MenuItem[] => node.children.map((child, index) => {
    let group = child;
    while (!group.path && group.children.length === 1)
      group = { ...group.children[0], label: `${group.label} · ${group.children[0].label}` };
    if (group.path) {
      const chosen = group.path;
      return { id: chosen, label: group.label, checked: chosen === path, run: () => choose(chosen) };
    }
    return { id: `${id}/${index}`, label: group.label, items: items(group, `${id}/${index}`) };
  });
  const menu: MenuItem[] = [{ id: 'none', label: none, checked: !path, run: () => choose('') }];
  for (const name of categories) {
    const root: Node = { label: category(name), children: [] };
    for (const parameter of parameters.filter(each => categoryOfPath(each.path) === name)) {
      const segments = label(parameter.path).split(' · ');
      let node = root;
      for (const segment of segments.slice(0, -1)) {
        let next = node.children.find(each => !each.path && each.label === segment);
        if (!next) {
          next = { label: segment, children: [] };
          node.children.push(next);
        }
        node = next;
      }
      node.children.push({ label: segments.at(-1)!, path: parameter.path, children: [] });
    }
    // A category whose names all begin alike (Equalizer · …, Dynamics · …) does not repeat it.
    const top = root.children.length === 1 && !root.children[0].path ? { ...root.children[0], label: root.label } : root;
    if (!top.children.length)
      continue;
    if (top.children.length === 1 && top.children[0].path) {
      const only = top.children[0].path;
      menu.push({ id: only, label: top.label, checked: only === path, run: () => choose(only) });
    }
    else
      menu.push({ id: name, label: top.label, items: items(top, name) });
  }
  return menu;
}

/**
 * The VCAs whose fader lanes a track's Fader lane shows, read only (p. 3909): those it is
 * assigned to, and the VCAs those are assigned to in turn, each once (§11.6, nested VCAs).
 */
export function vcaAncestors(timeline: Timeline, of: LanesOf): string[] {
  if (!('track' in of))
    return [];
  const track = timeline.tracks.find(row => row.kind === 'audio' && row.id === of.track) as { strip?: { vcas?: string[] } } | undefined;
  const seen: string[] = [], next = [...(track?.strip?.vcas ?? [])];
  while (next.length) {
    const id = next.shift()!;
    if (seen.includes(id))
      continue;
    seen.push(id);
    next.push(...(timeline.vcas?.find(vca => vca.id === id)?.vcas ?? []));
  }
  return seen;
}

/**
 * The new curve of a lane while it is written (p. 3907, 3910): red from where the control
 * began to be written to the playhead, at the values heard; blue under Preview. It follows the
 * playhead, so it is drawn again as the playhead moves, and the rest of the lane is not.
 */
function RecordingTrail({ target, lane, parameter, timeline, x, y }: {
  target: AutomationTarget; lane: Lane | undefined; parameter: AutomationParameter; timeline: Timeline;
  x: (sample: number) => number; y: (value: number) => number;
}) {
  const frame = useContext(AudioPlayhead), touch = useAutomationTouch();
  const trail = useRef<{ sample: number; value: number }[]>([]);
  const state = touch?.status(target), writing = Boolean(touch?.playing && (state === 'write' || state === 'preview'));
  const sample = Math.round(frame * timeline.rate.den / timeline.rate.num * timeline.sample_rate);
  const current = writing && touch
    ? { sample, value: touch.reading(target, laneValue(lane, sample / timeline.sample_rate, parameter.value), parameter) }
    : null;
  useLayoutEffect(() => {
    if (!current) {
      trail.current = [];
      return;
    }
    const last = trail.current.at(-1);
    if (last && current.sample < last.sample)
      trail.current = [];
    if (!last || last.sample !== current.sample || last.value !== current.value)
      trail.current.push(current);
  });
  if (!current)
    return null;
  const points = [...trail.current.filter(point => point.sample <= current.sample), current];
  const drawn = points.map((point, index) => {
    const step = parameter.kind === 'switch' && index ? `${x(point.sample)},${y(points[index - 1].value)} ` : '';
    return `${step}${x(point.sample)},${y(point.value)}`;
  }).join(' ');
  return <polyline
    className="automation-trail"
    points={drawn}
    fill="none"
    stroke={state === 'preview' ? '#69baff' : '#f66'}
    strokeWidth={2}
    pointerEvents="none"
  />;
}

/**
 * The lane needs the playhead only when it is used (snapping, paste, clear), so the playhead is
 * kept in a ref by this empty leaf: it is the only part drawn again as the playhead moves.
 */
function FollowPlayhead({ into }: { into: RefObject<number> }) {
  const frame = useContext(AudioPlayhead);
  useLayoutEffect(() => { into.current = frame; }, [into, frame]);
  return null;
}
export function AutomationLane(props: Props) {
  const { bridge, session, revision, timeline, of, lanes, locked, arm, scale, from, to, height, snapping, marks, path, pencil, onPath, onEdit, name } = props;
  const playhead = useRef(0);
  const t = useFairlightText(), touch = useAutomationTouch(), key = ownerKey(of);
  const failure = useFailureText();
  const { parameters, problem: parameterProblem } = useAutomationParameters(props, of);
  const [problem, setProblem] = useState<string | null>(null);
  const [selectedAt, setSelectedAt] = useState<number[]>([]), [range, setRange] = useState<[
    number,
    number
  ] | null>(null), [preview, setPreview] = useState<Lane | null>(null), [clear, setClear] = useState(false), [pointMenu, setPointMenu] = useState<{ index: number; x: number; y: number } | null>(null), [hint, setHint] = useState<string | null>(null), [temporaryPencil, setTemporaryPencil] = useState(false);
  const svg = useRef<SVGSVGElement>(null);
  const [parameterMenuAt, setParameterMenuAt] = useState<Menu | null>(null);
  const [armMenu, setArmMenu] = useState<{ x: number; y: number } | null>(null);
  // Arm as on the Mixer (p. 3901, 3907): a click arms this track or bus, Alt+click the selected
  // ones too, Ctrl+Alt+click every one; Safe and Off from the right-click menu, the same way.
  const setArm = (automation_arm: AutomationArm, modifiers: Modifiers) => {
    if ('vca' in of) {
      onEdit([{ op: 'set_vca', timeline: timeline.id, id: of.vca, patch: { automation_arm } }]);
      return;
    }
    const targets = operationTargets(mixerStrips(timeline), key, props.selection?.() ?? [], modifiers);
    if (targets.length)
      onEdit(targets.map(strip => stripChange(timeline.id, strip, { automation_arm })));
  };
  const drag = useRef<{
    kind: 'point' | 'range' | 'line' | 'pencil';
    sample: number;
    y: number;
    points: number[];
    lane?: Lane;
    delta: number;
    by: number;
    pencil?: Pencil;
    axis?: 'x' | 'y';
  } | null>(null);
  const gesture = useAudioGesture({
    bridge, session, revision, identity: `${timeline.id}/${key}/${path}`, locked: Boolean(locked), onFailure: e => setProblem(failure(e))
  });
  useEffect(() => {
    setSelectedAt([]);
    setRange(null);
  }, [session, timeline.id, key, path]);
  useEffect(() => {
    drag.current = null;
    setPreview(null);
  }, [session, timeline.id, key, path, revision]);
  const parameter = parameters.find(p => p.path === path), lane = lanes?.[path], drawn = preview ?? lane;
  // Points are selected by where they are, so that the selection outlasts an edit (of another
  // lane, or its own nudge) for as long as the points it holds are still there.
  const tickAt = (index: number) => tickOf(lane!.points[index].at, timeline.sample_rate);
  const selected = selectedAt.map(at => lane?.points.findIndex(p => tickOf(p.at, timeline.sample_rate) === at) ?? -1)
    .filter(index => index >= 0);
  const setSelected = (indices: number[]) => setSelectedAt(indices.filter(i => lane?.points[i]).map(tickAt));
  useEffect(() => {
    const present = new Set((lane?.points ?? []).map(p => tickOf(p.at, timeline.sample_rate)));
    setSelectedAt(old => old.every(at => present.has(at)) ? old : old.filter(at => present.has(at)));
  }, [lane, timeline.sample_rate]);
  const top = 22, bottom = height - 8, h = bottom - top;
  const x = (sample: number) => pixelAtSample(sample, scale, timeline) - from * scale;
  const y = (value: number) => top + (1 - valuePosition(value, parameter!)) * h;
  const sample = (event: {
    clientX: number;
  }) => {
    const px = event.clientX - (svg.current?.getBoundingClientRect().left ?? 0) + from * scale;
    let frame = px / scale;
    if (snapping) {
      const near = snap(0, [frame], snapCandidates(timeline, [], playhead.current, marks()), scale);
      frame = near.point ?? Math.round(frame);
    }
    return sampleAtPixel(frame * scale, scale, timeline);
  };
  const run = (change: Record<string, unknown>) => gesture.commit([{
    timeline: timeline.id, of, path, grouped: true, ...change
  } as Change]);
  const cancel = () => {
    drag.current = null;
    setPreview(null);
    setHint(null);
  };
  useEffect(() => {
    const keys = (event: globalThis.KeyboardEvent) => setTemporaryPencil(event.altKey && event.shiftKey);
    const blur = () => {
      cancel();
      setTemporaryPencil(false);
    };
    window.addEventListener('keydown', keys);
    window.addEventListener('keyup', keys);
    window.addEventListener('blur', blur);
    return () => {
      window.removeEventListener('keydown', keys);
      window.removeEventListener('keyup', keys);
      window.removeEventListener('blur', blur);
    };
  }, []);
  const down = (event: PointerEvent<SVGElement>, kind: 'point' | 'range' | 'line', index?: number) => {
    if (event.button !== 0 || locked || !parameter)
      return;
    event.preventDefault();
    event.stopPropagation();
    svg.current?.focus();
    svg.current?.setPointerCapture?.(event.pointerId);
    const at = sample(event), points = lane?.points ?? [];
    if (event.ctrlKey && event.altKey && index !== undefined) {
      run({ edit: 'lane_points', action: 'remove', at: [points[index].at] });
      return;
    }
    if (event.altKey && !event.shiftKey) {
      run({ edit: 'lane_points', action: 'add', points: [{ at: sampleTime(at, timeline.sample_rate) }] });
      return;
    }
    if (pencil || (event.altKey && event.shiftKey)) {
      const paint = new Pencil(timeline.sample_rate);
      paint.add(at, positionValue(1 - (event.clientY - (svg.current?.getBoundingClientRect().top ?? 0) - top) / h, parameter), performance.now(), true);
      drag.current = {
        kind: 'pencil', sample: at, y: event.clientY, points: [], lane, delta: 0, by: 0, pencil: paint
      };
      return;
    }
    let chosen = selected;
    if (index !== undefined) {
      if (event.ctrlKey)
        chosen = selected.includes(index) ? selected.filter(i => i !== index) : [...selected, index];
      else if (event.shiftKey && selected.length) {
        const anchor = selected[selected.length - 1];
        chosen = points.map((_, i) => i).filter(i => i >= Math.min(anchor, index) && i <= Math.max(anchor, index));
      }
      else if (!selected.includes(index))
        chosen = [index];
      setSelected(chosen);
    }
    else if (kind === 'line' && !range && !selected.length) {
      const next = points.findIndex(p => tickOf(p.at, timeline.sample_rate) > at);
      chosen = points.length ? next < 0 ? [points.length - 1] : next === 0 ? [0] : [next - 1, next] : [];
      setSelected(chosen);
    }
    else if (kind === 'range') {
      chosen = [];
      setSelected([]);
      setRange([at, at]);
    }
    drag.current = {
      kind, sample: at, y: event.clientY, points: chosen, lane, delta: 0, by: 0
    };
  };
  const move = (event: PointerEvent<SVGSVGElement>) => {
    const d = drag.current;
    if (!d || !parameter)
      return;
    event.stopPropagation();
    const at = sample(event), dy = d.y - event.clientY;
    if (d.kind === 'range') {
      const r: [
        number,
        number
      ] = [Math.min(at, d.sample), Math.max(at, d.sample)];
      setRange(r);
      setSelected((lane?.points ?? []).map((p, i) => ({ s: tickOf(p.at, timeline.sample_rate), i })).filter(p => p.s >= r[0] && p.s <= r[1]).map(p => p.i));
      return;
    }
    if (d.kind === 'pencil') {
      d.pencil!.add(at, positionValue(1 - (event.clientY - (svg.current?.getBoundingClientRect().top ?? 0) - top) / h, parameter), performance.now());
      setPreview({ kind: parameter.kind, scale: parameter.scale, points: d.pencil!.finish() });
      return;
    }
    if (event.shiftKey && d.kind === 'point' && !d.axis && (Math.abs(dy) > 3 || Math.abs(x(at) - x(d.sample)) > 3))
      d.axis = Math.abs(dy) > Math.abs(x(at) - x(d.sample)) ? 'y' : 'x';
    const original = d.lane?.points ?? [], base = d.points.length ? original[d.points[0]].value : parameter.value;
    let delta = positionValue(valuePosition(base, parameter) + dy / h * (event.shiftKey && d.kind === 'line' ? .1 : 1), parameter) - base;
    if (d.axis === 'x')
      delta = 0;
    if (path === 'fader_db')
      delta = Math.round(delta * 10) / 10;
    let by = d.kind === 'point' && d.axis !== 'y' ? at - d.sample : 0;
    const positions = original.map(p => tickOf(p.at, timeline.sample_rate));
    for (const i of d.points) {
      const previous = positions.slice(0, i).map((s, j) => ({ s, j })).filter(p => !d.points.includes(p.j)).at(-1)?.s ?? -1;
      const next = positions.map((s, j) => ({ s, j })).find(p => p.j > i && !d.points.includes(p.j))?.s ?? Number.MAX_SAFE_INTEGER;
      by = Math.max(previous + 1 - positions[i], Math.min(next - 1 - positions[i], by));
    }
    d.delta = delta;
    d.by = by;
    setHint(`${(base + delta).toFixed(2)} (${delta >= 0 ? '+' : ''}${delta.toFixed(2)})`);
    const previewPoints = original.map((p, i) => d.points.includes(i) ? { at: sampleTime(tickOf(p.at, timeline.sample_rate) + by, timeline.sample_rate), value: positionValue(valuePosition(p.value + delta, parameter), parameter) } : p);
    if (!original.length)
      previewPoints.push({ at: sampleTime(d.sample, timeline.sample_rate), value: positionValue(valuePosition(base + delta, parameter), parameter) });
    setPreview({
      ...d.lane, kind: parameter.kind, scale: parameter.scale, points: previewPoints
    });
  };
  const finish = (event: PointerEvent<SVGSVGElement>) => {
    const d = drag.current;
    cancel();
    if (!d)
      return;
    event.stopPropagation();
    svg.current?.releasePointerCapture?.(event.pointerId);
    if (d.kind === 'range')
      return;
    if (d.kind === 'pencil') {
      d.pencil!.add(sample(event), positionValue(1 - (event.clientY - (svg.current?.getBoundingClientRect().top ?? 0) - top) / h, parameter!), performance.now(), true);
      const points = d.pencil!.finish();
      if (points.length > 1)
        run({
          edit: 'lane_draw', from: points[0].at, to: points.at(-1)!.at, points
        });
      else if (points.length)
        run({ edit: 'lane_points', action: 'add', points });
      return;
    }
    if (!d.delta && !d.by)
      return;
    if (range && d.kind === 'line')
      run({
        edit: 'lane_range', from: sampleTime(range[0], timeline.sample_rate), to: sampleTime(range[1], timeline.sample_rate), action: 'shift_value', delta: d.delta
      });
    else if (d.points.length) {
      run({
        edit: 'lane_points', action: 'move', at: d.points.map(i => d.lane!.points[i].at), by: { ticks: d.by, scale: timeline.sample_rate }, delta: d.delta
      });
      setSelectedAt(d.points.map(i => tickOf(d.lane!.points[i].at, timeline.sample_rate) + d.by));
    }
    else
      onEdit([
        {
          edit: 'lane_points', timeline: timeline.id, of, path, action: 'add', points: [{ at: sampleTime(d.sample, timeline.sample_rate), value: parameter!.value + d.delta }], grouped: false
        },
        {
          edit: 'clear_lanes', timeline: timeline.id, of, paths: [path], then: 'hold', at: sampleTime(d.sample, timeline.sample_rate)
        },
      ]);
  };
  const execute = (key: Key): boolean => {
    const char = key.code === 'Comma' ? ',' : key.code === 'Period' ? '.' : key.key.toLowerCase(), points = lane?.points ?? [];
    const command = key.ctrl || key.meta;
    if ((command && ['a', 'x', 'c', 'v'].includes(char)) || ['delete', 'backspace', ',', '.'].includes(char)) {
      if (locked)
        return true;
      const chosen = selected.map(i => points[i]).filter(Boolean).sort((a, b) => tickOf(a.at, timeline.sample_rate) - tickOf(b.at, timeline.sample_rate)), a = range?.[0] ?? (chosen.length ? tickOf(chosen[0].at, timeline.sample_rate) : null), b = range?.[1] ?? (chosen.length ? tickOf(chosen.at(-1)!.at, timeline.sample_rate) : null);
      if (command && char === 'a') {
        setSelected(points.map((_, i) => i));
        return true;
      }
      if (command && (char === 'c' || char === 'x')) {
        if (a === null || b === null)
          return true;
        const picked = a === b ? chosen : [{ at: sampleTime(a, timeline.sample_rate), value: laneValue(lane, a / timeline.sample_rate, parameter?.value ?? 0) }, ...points.filter(p => tickOf(p.at, timeline.sample_rate) > a && tickOf(p.at, timeline.sample_rate) < b), { at: sampleTime(b, timeline.sample_rate), value: laneValue(lane, b / timeline.sample_rate, parameter?.value ?? 0) }];
        if (parameter)
          laneClipboard = {
            points: picked.map(p => ({ ...p, at: sampleTime(tickOf(p.at, timeline.sample_rate) - a, timeline.sample_rate) })),
            rate: timeline.sample_rate, path, parameter,
          };
        if (char === 'c')
          return true;
      }
      if (command && char === 'v') {
        const source = laneClipboard;
        if (!source?.points.length || !parameter)
          return true;
        const at = range?.[0] ?? Math.round(playhead.current * timeline.rate.den / timeline.rate.num * timeline.sample_rate);
        // Pasted on this lane, from another lane too: its times in this timeline's samples.
        const pasted = source.points.map(p => ({
          at: sampleTime(p.at.ticks * timeline.sample_rate / source.rate + at, timeline.sample_rate),
          value: pastedValue(p.value, source, path, parameter),
        }));
        if (pasted.length === 1)
          run({ edit: 'lane_points', action: 'add', points: pasted });
        else
          run({
            edit: 'lane_range', action: 'paste', from: pasted[0].at, to: pasted.at(-1)!.at, points: pasted
          });
        return true;
      }
      if ([',', '.'].includes(char)) {
        // A frame, or the subframe or millisecond chosen in the Automation Controls (p. 3797).
        const step = nudgeSamples(readNudge(), timeline);
        const by = step * (char === ',' ? -1 : 1) * (key.shift ? 5 : 1);
        if (chosen.length) {
          run({
            edit: 'lane_points', action: 'move', at: chosen.map(p => p.at), by: { ticks: by, scale: timeline.sample_rate },
          });
          setSelectedAt(old => old.map(at => at + by));
        }
        return true;
      }
      if (range && a !== null && b !== null && b > a)
        run({
          edit: 'lane_range', action: 'delete', from: sampleTime(a, timeline.sample_rate), to: sampleTime(b, timeline.sample_rate)
        });
      else if (chosen.length)
        run({ edit: 'lane_points', action: 'remove', at: chosen.map(p => p.at) });
      return true;
    }
    else if (key.key === 'Escape') {
      cancel();
      setRange(null);
      setSelected([]);
      return true;
    }
    return false;
  };
  useEffect(() => props.register?.(key, execute));
  const keys = (event: KeyboardEvent) => {
    const k = keyOf(event);
    if (editingAllowed(event.target, k) && execute(k)) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
  const poly = (curve: Lane | undefined) => {
    if (!parameter)
      return '';
    const points = curve?.points ?? [];
    const start = sampleAtPixel(from * scale, scale, timeline), end = sampleAtPixel(to * scale, scale, timeline);
    const visible = points.filter(p => tickOf(p.at, timeline.sample_rate) > start && tickOf(p.at, timeline.sample_rate) < end);
    const values = [{ at: sampleTime(start, timeline.sample_rate), value: laneValue(curve, start / timeline.sample_rate, parameter.value) }, ...visible, { at: sampleTime(end, timeline.sample_rate), value: laneValue(curve, end / timeline.sample_rate, parameter.value) }];
    return values.map((p, i) => `${curve?.kind === 'switch' && i ? `${x(tickOf(p.at, timeline.sample_rate))},${y(values[i - 1].value)} ` : ''}${x(tickOf(p.at, timeline.sample_rate))},${y(p.value)}`).join(' ');
  };
  const live = touch?.lives.find(v => sameTarget(v.target, { of, path })) ?? null;
  return <div
    className="automation-decoration"
    onKeyDown={keys}
    onPointerDownCapture={props.onActive}
    onFocusCapture={props.onActive}
  >
    <FollowPlayhead into={playhead} />

    <div
      className="automation-lane-header"
      style={{ width: HEADER_WIDTH, display: height < 96 ? 'none' : undefined }}
      onPointerDown={e => e.stopPropagation()}
      onContextMenu={e => {
        e.preventDefault();
        e.stopPropagation();
        setClear(true);
      }}
    >

      <button
        type="button"
        className="automation-parameter"
        aria-label={`${t('Automation parameter')} ${name}`}
        aria-haspopup="menu"
        aria-expanded={Boolean(parameterMenuAt)}
        onClick={event => {
          const box = event.currentTarget.getBoundingClientRect();
          setParameterMenuAt({
            x: box.left, y: box.bottom, label: `${t('Automation parameter')} ${name}`,
            items: parameterMenu(parameters, each => parameterLabel(timeline, of, each, t), category => t(category), path, onPath, t('None')),
          });
        }}
      >
        {path ? parameterLabel(timeline, of, path, t) : t('None')} ▾
      </button>

      {parameterMenuAt && <CoveringMenu menu={parameterMenuAt} onClose={() => setParameterMenuAt(null)} />}

      <button
        type="button"
        className="automation-arm"
        aria-label={`${t('Arm')} ${name}`}
        aria-pressed={arm === true}
        data-arm={arm === 'safe' || arm === 'off' ? arm : undefined}
        data-writing={touch?.writing(of) ?? undefined}
        disabled={locked}
        title={`${t('Arm')}: Alt, Ctrl+Alt`}
        onClick={event => setArm(arm !== true, event)}
        onContextMenu={event => {
          event.preventDefault();
          event.stopPropagation();
          if (!locked)
            setArmMenu({ x: event.clientX, y: event.clientY });
        }}
      >
        {t(arm === 'safe' ? 'Safe' : arm === 'off' ? 'Bypass' : 'Arm')}
      </button>

      {armMenu && <PortalMenu anchor={armMenu} label={`${t('Arm')} ${name}`} onClose={() => setArmMenu(null)}>
        {([['safe', 'Safe'], ['off', 'Bypass'], [false, 'Unarmed']] as const).map(([value, label]) => <button
          key={String(value)}
          type="button"
          role="menuitem"
          onClick={event => {
            setArm(value, event);
            setArmMenu(null);
          }}
        >
          {t(label)}
        </button>)}
        <button type="button" role="menuitem" onClick={() => setArmMenu(null)}>{t('Cancel')}</button>
      </PortalMenu>}

      {clear && <div role="dialog" aria-label={t('Clear Track Automation')}>
        {(['reset', 'hold'] as const).map(then => <button
          disabled={locked}
          key={then}
          onClick={() => {
            onEdit([{
              edit: 'clear_lanes', timeline: timeline.id, of, then, at: sampleTime(playhead.current * timeline.rate.den / timeline.rate.num * timeline.sample_rate, timeline.sample_rate)
            }]);
            setClear(false);
          }}
        >
          {t(then === 'reset' ? 'Reset' : 'Hold')}
        </button>)}
        <button onClick={() => setClear(false)}>{t('Cancel')}</button>
      </div>}

      {pointMenu !== null && <PortalMenu anchor={pointMenu} label={`${name} ${parameterLabel(timeline, of, path, t)}`}
        onClose={() => setPointMenu(null)}>
        <button type="button" role="menuitem" onClick={() => {
          const p = lane?.points[pointMenu.index];
          if (p && !locked)
            run({ edit: 'lane_points', action: 'remove', at: [p.at] });
          setPointMenu(null);
        }}>
          {t('Delete')}
        </button>
        <button type="button" role="menuitem" onClick={() => setPointMenu(null)}>{t('Cancel')}</button>
      </PortalMenu>}
    </div>

    {path && parameter && <svg
      ref={svg}
      tabIndex={0}
      aria-label={`${name} ${parameterLabel(timeline, of, path, t)}`}
      className="automation-lane"
      style={{ left: HEADER_WIDTH + from * scale, width: (to - from) * scale, height }}
      onPointerMove={move}
      onPointerUp={finish}
      onPointerCancel={cancel}
      onLostPointerCapture={cancel}
    >

      <rect
        x={0}
        y={0}
        width="100%"
        height={pencil || temporaryPencil ? height : height / 2}
        fill="transparent"
        pointerEvents="all"
        onPointerDown={e => down(e, 'range')}
      />

      {range && <rect
        x={x(range[0])}
        y={top}
        width={Math.max(1, x(range[1]) - x(range[0]))}
        height={h}
        fill="#58b9ff22"
        pointerEvents="none"
      />}

      {preview && <polyline points={poly(lane)} fill="none" stroke="#777" />}

      <polyline
        points={poly(drawn)}
        fill="none"
        stroke={live ? '#f66' : preview || lane?.points.length ? '#6ddd92' : '#aaa'}
        strokeWidth={2}
        pointerEvents="none"
      />

      <polyline
        points={poly(drawn)}
        fill="none"
        stroke="transparent"
        strokeWidth={12}
        pointerEvents="stroke"
        onPointerDown={e => down(e, 'line')}
      />

      {(drawn?.points ?? []).map((p, i) => <circle
        key={i}
        cx={x(tickOf(p.at, timeline.sample_rate))}
        cy={y(p.value)}
        r={selected.includes(i) ? 5 : 3.5}
        fill={selected.includes(i) ? '#fff' : '#6ddd92'}
        pointerEvents="all"
        onPointerDown={e => down(e, 'point', i)}
        onContextMenu={e => {
          e.preventDefault();
          e.stopPropagation();
          if (!locked)
            setPointMenu({ index: i, x: e.clientX, y: e.clientY });
        }}
      />)}

      {path === 'fader_db' && vcaAncestors(timeline, of).map(id => {
        const v = timeline.vcas?.find(v => v.id === id);
        return v?.lanes?.fader_db && <polyline
          key={id}
          className="automation-vca-line"
          points={poly(v.lanes.fader_db)}
          stroke="#daa9ff"
          fill="none"
          strokeDasharray="4 3"
          pointerEvents="none"
        />;
      })}

      <RecordingTrail target={{ of, path }} lane={lane} parameter={parameter} timeline={timeline} x={x} y={y} />

      {live && <line
        x1={0}
        x2="100%"
        y1={y(live.value)}
        y2={y(live.value)}
        stroke={touch?.preview ? '#69baff' : '#f66'}
        pointerEvents="none"
      />}
    </svg>}

    {hint && <output className="automation-drag-value">{hint}</output>}

    {(problem || parameterProblem) && <span role="alert" className="automation-problem">{problem || parameterProblem}</span>}
  </div>;
}
