import { useCallback, useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type AudioAnalysis, type LoudnessReading, type Meters } from '../engine';

type Bridge = Pick<EngineBridge, 'call' | 'onEvent'>;
interface JobOwner { live: boolean; waiting: boolean; running: boolean; cancelled: boolean; job: string | null; early: Map<string, AudioAnalysis> }
/** Analysis events are broadcast; only the job acknowledged to this page belongs to it. */
export function useAudioAnalysis(bridge: Bridge, session: number, timeline: string | null) {
  const [analysis, setAnalysis] = useState<AudioAnalysis | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const owner = useRef<JobOwner>({ live: false, waiting: false, running: false, cancelled: false, job: null, early: new Map() });
  const cancelJob = (job: string) => void callEngine(bridge, 'audio.cancel', { job }, { expectedSession: session }).catch(() => { /* Completion may have won the cancellation race. */ });
  useEffect(() => {
    const current: JobOwner = { live: true, waiting: false, running: false, cancelled: false, job: null, early: new Map() };
    owner.current = current; setAnalysis(null); setProblem(null); setPending(false);
    const off = bridge.onEvent(event => {
      if (!current.live || event.kind !== 'notification' || event.session !== session || event.method !== 'audio.analysis') return;
      const update = event.params as AudioAnalysis;
      if (current.job === update.job && !current.cancelled) { setAnalysis(update); current.running = update.state === 'running'; setPending(current.running); }
      else if (current.waiting) current.early.set(update.job, update);
    });
    return () => { current.live = false; if (current.running && current.job) cancelJob(current.job); current.early.clear(); off(); };
  }, [bridge, session, timeline]);
  const start = useCallback(async (method: 'audio.analyze' | 'audio.level_match', params: Record<string, unknown>) => {
    const current = owner.current;
    if (!timeline || !current.live || current.waiting || current.running) return;
    current.waiting = true; current.running = true; current.cancelled = false; current.job = null; current.early.clear(); setPending(true); setProblem(null); setAnalysis(null);
    try {
      const { job } = await callEngine(bridge, method, { timeline, ...params }, { expectedSession: session });
      if (!current.live || current.cancelled) { cancelJob(job); return; }
      current.job = job;
      const update = current.early.get(job) ?? { job, state: 'running' as const, done: 0, total: 0 };
      current.running = update.state === 'running'; setAnalysis(update); setPending(current.running);
    } catch (error) {
      current.running = false;
      if (current.live) { setProblem(error instanceof Error ? error.message : String(error)); setPending(false); }
    } finally { current.waiting = false; current.early.clear(); }
  }, [bridge, timeline, session]);
  const cancel = () => {
    const current = owner.current; current.cancelled = true; current.running = false;
    if (current.job) cancelJob(current.job);
    setPending(false); setAnalysis({ job: current.job ?? '', state: 'cancelled' });
  };
  return { analysis, problem, pending, start, cancel };
}

/** The protocol polls meters; no subscribe RPC exists. One in-flight read, at most 30Hz. */
export function useAudioReadings(bridge: Bridge, session: number, timeline: string | null, visible: boolean) {
  const [meters, setMeters] = useState<Meters | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const lifecycle = useRef<{ live: boolean; action: number }>({ live: false, action: 0 });
  const jobs = useAudioAnalysis(bridge, session, timeline);
  useEffect(() => {
    const current = { live: true, action: 0 }; lifecycle.current = current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let polling = false;
    setMeters(null); setProblem(null);
    const read = async () => {
      if (!current.live || !visible || !timeline || document.hidden || polling) return;
      polling = true;
      const action = current.action;
      try {
        const next = await callEngine(bridge, 'audio.meters', { timeline }, { expectedSession: session });
        if (current.live && action === current.action && next.timeline === timeline) { setMeters(next); setProblem(null); }
      } catch (error) { if (current.live) setProblem(error instanceof Error ? error.message : String(error)); }
      finally {
        polling = false;
        if (current.live && visible && !document.hidden) timer = setTimeout(() => void read(), 34);
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
      if (current.live && sequence === current.action) setMeters(old => old ? { ...old, loudness } : old);
    } catch (error) { if (current.live) setProblem(error instanceof Error ? error.message : String(error)); }
  };
  const resetPeaks = async () => {
    const current = lifecycle.current;
    if (!timeline || !current.live) return;
    const sequence = ++current.action;
    try {
      const next = await callEngine(bridge, 'audio.meters', { timeline, reset: true }, { expectedSession: session });
      if (current.live && sequence === current.action && next.timeline === timeline) setMeters(next);
    } catch (error) { if (current.live) setProblem(error instanceof Error ? error.message : String(error)); }
  };
  return { meters, problem: problem ?? jobs.problem, analysis: jobs.analysis, pending: jobs.pending, action, resetPeaks,
    cancel: jobs.cancel, analyze: (clips: string[], standard: string) => jobs.start('audio.analyze', { targets: clips.map(clip => ({ clip })), standard }) };
}
