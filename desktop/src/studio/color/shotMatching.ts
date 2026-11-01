import type { EngineBridge } from '../../bridge';
import { callEngine, type Clip, type GradeNode, type Op, type ShotMatch, type Target } from '../engine';
import { makeDefaultNode } from './nodeEdits';
import { figures, solve, unmapped, type Match } from './shotmatch';

export interface MatchRequest {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>;
  session: number;
  revision: number;
  timeline: string;
  reference: Clip;
  clips: readonly Clip[];
  nodes: Readonly<Record<string, string>>;
  signal?: AbortSignal;
}

export class ShotMatchError extends Error {
  constructor(readonly reason: 'wrongNode' | 'locked' | 'animated' | 'cancelled' | 'cleanup', options?: ErrorOptions) {
    super(reason, options);
  }
}

interface Plan {
  clip: Clip;
  target: Target;
  node: GradeNode | null;
  match: Match | null;
}

let gestures = 0;
const channels = ['r', 'g', 'b'] as const;
const rounded = (x: number) => Math.round(x * 1e6) / 1e6;
const roundedMatch = (match: Match): Match => ({
  gamma: { r: rounded(match.gamma.r), g: rounded(match.gamma.g), b: rounded(match.gamma.b) },
  gain: { r: rounded(match.gain.r), g: rounded(match.gain.g), b: rounded(match.gain.b) },
  offset: { r: rounded(match.offset.r), g: rounded(match.offset.g), b: rounded(match.offset.b) },
  contrast: rounded(match.contrast), pivot: rounded(match.pivot),
});

function fixedMatch(match: ShotMatch | null | undefined): Match | null {
  if (match == null) return null;
  if (typeof match.contrast !== 'number' || typeof match.pivot !== 'number'
    || ['gamma', 'gain', 'offset'].some(key => channels.some(channel => typeof match[key as 'gamma' | 'gain' | 'offset'][channel] !== 'number'))) {
    throw new ShotMatchError('animated');
  }
  return match as Match;
}

/** Match a snapshot, never a later revision of the project, as one shared undo step (RM20 pp. 3047–3050). */
export async function matchShots(request: MatchRequest): Promise<void> {
  const { bridge, timeline, revision, reference, signal } = request;
  const plan = (clip: Clip, node: GradeNode | null, matching: boolean): Plan => {
    if (matching && node?.kind !== 'corrector' && node !== null) throw new ShotMatchError('wrongNode');
    if (matching && node?.locked) throw new ShotMatchError('locked');
    return { clip, target: { level: 'clip', timeline, clip: clip.id }, node,
      match: matching && node?.kind === 'corrector' ? fixedMatch(node.params.shot_match) : null };
  };
  const plans = request.clips.filter(clip => clip.id !== reference.id).map(clip => plan(clip,
    clip.grade.nodes.find(node => node.id === request.nodes[`clip:${clip.id}`]) ?? clip.grade.nodes[0] ?? null, true));
  if (plans.length === 0) return;
  const output = reference.grade.links.find(link => link.kind === 'rgb' && link.to === 'output' && link.input === 0)?.from;
  const probe = plan(reference, reference.grade.nodes.find(node => node.id === output) ?? null, false);
  const adding = [...plans, probe].filter(each => each.node === null);
  const originals: Op[] = adding.map(each => ({ op: 'set_grade', grade: each.target, value: each.clip.grade }));
  const gesture = adding.length ? { id: `shot-match-${++gestures}` } : undefined;
  let sameSession = true;
  let started = false;
  let timer: ReturnType<typeof setInterval> | undefined;
  let pending: Promise<void> | null = null;
  let keepFailure: unknown;
  let stopWaiting: (() => void) | null = null;
  const off = bridge.onEvent(event => {
    if ((event.kind === 'opened' && event.session !== request.session)
      || (event.kind === 'exited' && event.session === request.session)) {
      sameSession = false;
      stopWaiting?.();
    }
  });
  const check = () => {
    if (!sameSession || signal?.aborted) throw new ShotMatchError('cancelled');
    if (keepFailure !== undefined) throw keepFailure;
  };
  const stop = async () => { clearInterval(timer); await pending; };
  const waitingFor = async <T,>(read: Promise<T>): Promise<T> => {
    let cancel!: () => void;
    const cancelled = new Promise<never>((_, reject) => { cancel = () => reject(new ShotMatchError('cancelled')); });
    stopWaiting = cancel;
    signal?.addEventListener('abort', cancel, { once: true });
    if (signal?.aborted || !sameSession) cancel();
    try {
      // A view draw cannot be stopped, but rollback need not wait for it: it uses the main
      // connection. Promise.race also consumes a late read rejection after cancellation.
      return await Promise.race([read, cancelled]);
    } finally {
      signal?.removeEventListener('abort', cancel);
      stopWaiting = null;
    }
  };
  const apply = (ops: readonly Op[], end = false) => callEngine(bridge, 'transaction.apply', {
    // Keep the revision before the first transaction. After a timeout, undo or other edit ends
    // this gesture, this is stale and cannot silently start a second history step.
    revision, ops, ...(gesture ? { gesture: end ? { ...gesture, end: true } : gesture } : {}),
  });
  try {
    check();
    if (gesture) {
      const first = await callEngine(bridge, 'transaction.apply', { revision, gesture, ops: adding.map(each => makeDefaultNode(each.target)) });
      started = true;
      for (const each of adding) {
        const added = first.done.find((op): op is Extract<Op, { op: 'add_node' }> => op.op === 'add_node'
          && op.grade.level === 'clip' && op.grade.clip === each.clip.id && op.grade.timeline === timeline);
        if (!added) throw new Error('The engine did not return the default node it added.');
        each.node = added.node;
      }
      check();
      // The engine expires idle gestures after 10 seconds. Statistics use the separate view
      // connection, so a long draw cannot block this same-value transaction on the main one.
      const held = adding[0];
      timer = setInterval(() => {
        if (pending !== null || keepFailure !== undefined || !sameSession || signal?.aborted) return;
        pending = apply([{ op: 'set_node', grade: held.target, id: held.node!.id, label: held.node!.label }])
          .then(() => {}, error => { keepFailure = error; }).finally(() => { pending = null; });
      }, 3000);
    }
    const measure = async (each: Plan) => {
      check();
      const read = await waitingFor(callEngine(bridge, 'scopes.statistics', {
        timeline, frame: each.clip.start + Math.floor(each.clip.length / 2), grade: each.target, node: each.node!.id, bins: 1024,
      }));
      check();
      const measured = figures(read.histogram, [read.red.mean, read.green.mean, read.blue.mean]);
      return each.match ? unmapped(measured, each.match) : measured;
    };
    // The reference is measured as it looks now, including an earlier Shot Match. Only the
    // nodes receiving a replacement match are measured with their previous match removed.
    const wanted = await measure(probe);
    const ops: Op[] = [];
    for (const each of plans) {
      const solved = solve(await measure(each), wanted);
      ops.push({ op: 'set_node', grade: each.target, id: each.node!.id, params: { shot_match: roundedMatch(solved.match) } });
    }
    if (adding.includes(probe)) ops.push({ op: 'set_grade', grade: probe.target, value: reference.grade });
    await stop(); check();
    await apply(ops, true);
    started = false;
  } catch (error) {
    await stop();
    if (started && sameSession) {
      try { await apply(originals, true); }
      catch { throw new ShotMatchError('cleanup', { cause: error }); }
    }
    throw error;
  } finally {
    clearInterval(timer); off();
  }
}
