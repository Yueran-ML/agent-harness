import { useCallback, useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, EngineError, type AudioAnalysis, type LoudnessReading, type MeterChannel, type Meters } from '../engine';

type Bridge = Pick<EngineBridge, 'call' | 'onEvent'>;
interface JobOwner { live: boolean; waiting: boolean; running: boolean; cancelled: boolean; job: string | null; early: Map<string, AudioAnalysis> }
const sameChannel = (a: MeterChannel | null, b: MeterChannel | null) => a === b || Boolean(a && b
  && a.bar === b.bar && a.peak === b.peak && a.bar_position === b.bar_position && a.peak_position === b.peak_position);
function sameMeters(a: Meters | null, b: Meters): boolean {
  return a !== null && a.timeline === b.timeline && a.playing === b.playing && a.rate === b.rate
    && a.scale === b.scale && a.lost === b.lost && sameChannel(a.control_room, b.control_room)
    && (Object.keys(b.loudness) as (keyof LoudnessReading)[]).every(key => a.loudness[key] === b.loudness[key])
    && a.strips.length === b.strips.length && a.strips.every((strip, index) => {
      const next = b.strips[index];
      const owner = 'track' in strip.owner ? 'track' in next.owner && strip.owner.track === next.owner.track
        : 'bus' in next.owner && strip.owner.bus === next.owner.bus;
      return owner && (['pre', 'post'] as const).every(point => strip[point].length === next[point].length
        && strip[point].every((channel, i) => sameChannel(channel, next[point][i])));
    });
}
/** Analysis events are broadcast; only the job acknowledged to this page belongs to it. */
export function useAudioAnalysis(bridge: Bridge, session: number, timeline: string | null, identity = timeline) {
  const [analysis, setAnalysis] = useState<AudioAnalysis | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const owner = useRef<JobOwner>({ live: false, waiting: false, running: false, cancelled: false, job: null, early: new Map() });
  const cancelJob = (job: string, current?: JobOwner) => void callEngine(bridge, 'audio.cancel', { job }, { expectedSession: session }).catch(error => {
    // A completed job is already missing: its terminal event, not this reply, settles the result.
    if (error instanceof EngineError && (error.data as { reason?: string } | undefined)?.reason === 'missing') return;
    if (current?.live && current.running && current.job === job) {
      current.cancelled = false;
      setProblem(error instanceof Error ? error.message : String(error));
    }
  });
  useEffect(() => {
    const current: JobOwner = { live: true, waiting: false, running: false, cancelled: false, job: null, early: new Map() };
    owner.current = current; setAnalysis(null); setProblem(null); setPending(false);
    const off = bridge.onEvent?.(event => {
      if (!current.live || event.kind !== 'notification' || event.session !== session || event.method !== 'audio.analysis') return;
      const update = event.params as AudioAnalysis;
      if (current.job === update.job && current.running) {
        setAnalysis(update); current.running = update.state === 'running'; setPending(current.running);
        if (!current.running) setProblem(null);
      }
      else if (current.waiting) current.early.set(update.job, update);
    });
    return () => { current.live = false; if (current.running && current.job) cancelJob(current.job); current.early.clear(); off?.(); };
  }, [bridge, session, timeline, identity]);
  const start = useCallback(async (method: 'audio.analyze' | 'audio.level_match' | 'audio.level_profile' | 'audio.normalize' | 'audio.noise_learn', params: Record<string, unknown>) => {
    const current = owner.current;
    if (!timeline || !current.live || current.waiting || current.running) return;
    current.waiting = true; current.running = true; current.cancelled = false; current.job = null; current.early.clear(); setPending(true); setProblem(null); setAnalysis(null);
    try {
      const { job } = await callEngine(bridge, method, { timeline, ...params }, { expectedSession: session });
      if (!current.live) { cancelJob(job); return; }
      current.job = job;
      const update = current.early.get(job) ?? { job, state: 'running' as const, done: 0, total: 0 };
      current.running = update.state === 'running'; setAnalysis(update); setPending(current.running);
      if (current.cancelled && current.running) cancelJob(job, current);
    } catch (error) {
      current.running = false;
      if (current.live) { setProblem(error instanceof Error ? error.message : String(error)); setPending(false); }
    } finally { current.waiting = false; current.early.clear(); }
  }, [bridge, timeline, session]);
  const cancel = () => {
    const current = owner.current;
    if (!current.live || !current.running || current.cancelled) return;
    current.cancelled = true;
    if (current.job) cancelJob(current.job, current);
    // A request to cancel is not a terminal event: completion may already have won.
  };
  return { analysis, problem, pending, start, cancel };
}

/** The protocol polls meters; no subscribe RPC exists. One in-flight read, at most 30Hz. */
export function useAudioReadings(bridge: Bridge, session: number, timeline: string | null, visible: boolean) {
  const [meters, setMeters] = useState<Meters | null>(null);
  const latestMeters = useRef<Meters | null>(null);
  const publish = (next: Meters) => {
    if (sameMeters(latestMeters.current, next)) return false;
    latestMeters.current = next;
    setMeters(next);
    return true;
  };
  const [problem, setProblem] = useState<string | null>(null);
  const lifecycle = useRef<{ live: boolean; action: number }>({ live: false, action: 0 });
  const jobs = useAudioAnalysis(bridge, session, timeline);
  useEffect(() => {
    const current = { live: true, action: 0 }; lifecycle.current = current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let polling = false;
    latestMeters.current = null; setMeters(null); setProblem(null);
    const read = async () => {
      if (!current.live || !visible || !timeline || document.hidden || polling) return;
      polling = true;
      const action = current.action;
      let delay = 100;
      try {
        const next = await callEngine(bridge, 'audio.meters', { timeline }, { expectedSession: session });
        if (current.live && action === current.action && next.timeline === timeline) {
          const changed = publish(next);
          delay = next.playing || changed ? 34 : 100;
          setProblem(null);
        }
      } catch (error) { if (current.live) setProblem(error instanceof Error ? error.message : String(error)); }
      finally {
        polling = false;
        if (current.live && visible && !document.hidden) timer = setTimeout(() => void read(), delay);
      }
    };
    const visibility = () => { clearTimeout(timer); if (!document.hidden) void read(); };
    document.addEventListener('visibilitychange', visibility); void read();
    return () => { current.live = false; clearTimeout(timer); document.removeEventListener('visibilitychange', visibility); };
  }, [bridge, session, timeline, visible]);
  const action = async (which: 'start' | 'pause' | 'reset') => {
    const current = lifecycle.current;
    if (!timeline || !current.live) return;
    const sequence = ++current.action;
    try {
      const loudness: LoudnessReading = await callEngine(bridge, 'audio.loudness', { timeline, action: which }, { expectedSession: session });
      if (current.live && sequence === current.action && latestMeters.current) publish({ ...latestMeters.current, loudness });
    } catch (error) { if (current.live) setProblem(error instanceof Error ? error.message : String(error)); }
  };
  const resetPeaks = async () => {
    const current = lifecycle.current;
    if (!timeline || !current.live) return;
    const sequence = ++current.action;
    try {
      const next = await callEngine(bridge, 'audio.meters', { timeline, reset: true }, { expectedSession: session });
      if (current.live && sequence === current.action && next.timeline === timeline) publish(next);
    } catch (error) { if (current.live) setProblem(error instanceof Error ? error.message : String(error)); }
  };
  return { meters, problem: problem ?? jobs.problem, analysis: jobs.analysis, pending: jobs.pending, action, resetPeaks,
    cancel: jobs.cancel, analyze: (clips: string[], standard: string) => jobs.start('audio.analyze', { targets: clips.map(clip => ({ clip })), standard }) };
}
