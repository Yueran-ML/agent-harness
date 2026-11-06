import { useContext, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type RefObject } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type AutomationArm, type Lane, type Lanes, type LanesOf, type Timeline } from '../engine';
import { useAutomationParameters } from '../audio/useAutomationParameters';
import type { Change } from '../timeline/commands';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { HEADER_WIDTH } from '../timeline/geometry';
import { AudioPlayhead } from '../timeline/AudioOverlay';
import { snap, snapCandidates, type Marks } from '../timeline/snap';
import { editingAllowed } from '../keys';
import { keyOf, type Key } from '../../app/keys';
import { categories, categoryOfPath, laneValue, ownerKey, Pencil, pixelAtSample, positionValue, sampleAtPixel, sampleTime, tickOf, valuePosition } from './automation';
import { useAutomationTouch, sameTarget } from './AutomationTouch';
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
  onActive?: () => void;
  onPath: (path: string) => void;
  onEdit: (ops: Change[]) => void;
  name: string;
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
  const svg = useRef<SVGSVGElement>(null), clip = useRef<Lane['points']>([]);
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
        clip.current = picked.map(p => ({ ...p, at: sampleTime(tickOf(p.at, timeline.sample_rate) - a, timeline.sample_rate) }));
        if (char === 'c')
          return true;
      }
      if (command && char === 'v') {
        if (!clip.current.length)
          return true;
        const at = range?.[0] ?? Math.round(playhead.current * timeline.rate.den / timeline.rate.num * timeline.sample_rate);
        const pasted = clip.current.map(p => ({ ...p, at: sampleTime(p.at.ticks + at, timeline.sample_rate) }));
        if (pasted.length === 1)
          run({ edit: 'lane_points', action: 'add', points: pasted });
        else
          run({
            edit: 'lane_range', action: 'paste', from: pasted[0].at, to: pasted.at(-1)!.at, points: pasted
          });
        return true;
      }
      if ([',', '.'].includes(char)) {
        const step = Math.round(timeline.sample_rate * timeline.rate.den / timeline.rate.num);
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

      <select
        aria-label={`${t('Automation parameter')} ${name}`}
        value={path}
        onChange={e => onPath(e.target.value)}
      >
        <option value="">{t('None')}</option>
        {categories.map(category => <optgroup key={category} label={t(category)}>
          {parameters.filter(p => categoryOfPath(p.path) === category).map(p => <option value={p.path} key={p.path}>{parameterLabel(timeline, of, p.path, t)}</option>)}
        </optgroup>)}
      </select>

      <select
        aria-label={`${t('Arm')} ${name}`}
        disabled={locked}
        value={arm === true ? 'armed' : arm === false || arm === undefined ? 'unarmed' : arm}
        onChange={e => {
          const v = e.target.value;
          const automation_arm = v === 'armed' ? true : v === 'unarmed' ? false : v;
          onEdit(['track' in of ? {
            edit: 'set_track_audio', timeline: timeline.id, track: of.track, patch: { automation_arm }
          } : 'bus' in of ? {
            op: 'set_bus', timeline: timeline.id, id: of.bus, patch: { automation_arm }
          } : {
            op: 'set_vca', timeline: timeline.id, id: of.vca, patch: { automation_arm }
          }]);
        }}
      >
        {([['unarmed', 'Unarmed'], ['armed', 'Arm'], ['safe', 'Safe'], ['off', 'Bypass']] as const).map(([v, label]) => <option key={v} value={v}>{t(label)}</option>)}
      </select>

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

      {path === 'fader_db' && 'track' in of && (timeline.tracks.find(tr => 'track' in of && tr.id === of.track && tr.kind === 'audio') as import('../engine').ClipTrack | undefined)?.strip?.vcas?.map(id => {
        const v = timeline.vcas?.find(v => v.id === id);
        return v?.lanes?.fader_db && <polyline
          key={id}
          points={poly(v.lanes.fader_db)}
          stroke="#daa9ff"
          fill="none"
          strokeDasharray="4 3"
          pointerEvents="none"
        />;
      })}

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
