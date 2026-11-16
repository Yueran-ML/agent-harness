import { useEffect, useRef } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, EngineError } from '../engine';

/**
 * What the engine was asked last for this page, and what came of it: the clip and the node (null
 * for what comes out of the composition) of a view that is `on` or was `refused`, or a view turned
 * `off`; and whether the state of the monitor has told the view since it was asked for.
 */
interface Asked {
  bridge: Pick<EngineBridge, 'call'>; session: number; clip: string | null; node: string | null;
  state: 'on' | 'off' | 'refused'; confirmed: boolean;
}

/** Why the engine refused a call, where it says. */
const reasonOf = (error: unknown): string | null => {
  const data = error instanceof EngineError && typeof error.data === 'object' && error.data !== null ? error.data as { reason?: unknown } : null;
  return typeof data?.reason === 'string' ? data.reason : null;
};

/**
 * The Fusion view of the timeline monitor, kept as the Fusion page has it (`viewer.fusion`,
 * PROTOCOL.md, "The Fusion view"): while the page is shown and has a clip, the monitor shows the
 * picture of the node that is viewed of the composition of that clip, alone, in place of the frame
 * of the timeline; `node` null is what comes out of the composition (the `MediaOut`).
 *
 * - The engine is asked only when the clip or the node is another, or the view is to be turned on
 *   or off: not at each frame, and not at each edit.
 * - The page left (another page, another timeline, another engine), or no clip to show: the view is
 *   turned off (`{clip: null}`), where this page has it on.
 * - What the engine refuses changes nothing of what the monitor shows. A node that it has not (it
 *   was taken away) is told to `onNodeGone`, which goes back to the output; a clip that it has not,
 *   a clip of sound, or a monitor with no timeline turns off the view of the clip before, where
 *   there was one, and is not asked for again until another clip or node is to be shown. Neither is
 *   said to the user: the page shows what it can.
 * - The engine lets go of the view when the monitor loads a timeline. Where the state had told the
 *   view of this clip (`told`) and tells none any more, though the page still wants it, the engine
 *   is asked once more.
 */
export function useFusionView({ bridge, session, on, clip, node, told, onNodeGone }: {
  bridge: Pick<EngineBridge, 'call'>; session: number;
  /** Whether the monitor can show the view: the shell lays the engine's view over the viewer, and the timeline monitor has the timeline of the page loaded. */
  on: boolean;
  /** The clip whose composition the page shows, by its id, or null. */
  clip: string | null;
  /** The node that is viewed, by its id; null for what comes out of the composition. */
  node: string | null;
  /** The clip of the Fusion view as the state of the monitor tells it, or null while it tells none. */
  told: string | null;
  onNodeGone: () => void;
}): void {
  const asked = useRef<Asked | null>(null);
  const now = useRef({ onNodeGone });
  now.current = { onNodeGone };
  const wanted = on ? clip : null;

  useEffect(() => {
    const known = asked.current;
    const last = known !== null && known.bridge === bridge && known.session === session ? known : null;
    const off = () => { void callEngine(bridge, 'viewer.fusion', { clip: null }, { expectedSession: session }).catch(() => {}); };
    if (wanted === null) {
      // Nothing to show: the view is turned off where this page has it on, and nothing is asked where it has not.
      if (last === null || last.state !== 'on') return;
      asked.current = { bridge, session, clip: null, node: null, state: 'off', confirmed: false };
      off();
      return;
    }
    if (last !== null && last.clip === wanted && last.node === node) {
      // What the engine refused is not asked for again as it is.
      if (last.state === 'refused') return;
      if (last.state === 'on') {
        // The state tells the view that was asked for.
        if (told === wanted) { last.confirmed = true; return; }
        // It told it, and tells none now: the engine let go of the view (the monitor loaded a timeline). It is asked again, once.
        if (!(last.confirmed && told === null)) return;
      }
    }
    const wasOn = last !== null && last.state === 'on';
    // A view of this clip that the state tells already (another node of it was viewed) is told as soon as it is asked for.
    const mine: Asked = { bridge, session, clip: wanted, node, state: 'on', confirmed: told === wanted };
    asked.current = mine;
    void callEngine(bridge, 'viewer.fusion', node === null ? { clip: wanted } : { clip: wanted, node }, { expectedSession: session }).catch((error) => {
      // An answer to a question that is asked no longer says nothing of the one that is.
      if (asked.current !== mine) return;
      if (reasonOf(error) === 'node' && node !== null) {
        // The node is gone, and the monitor shows what it showed before: the page goes back to the output, which is asked for where it is not that.
        asked.current = last;
        now.current.onNodeGone();
        return;
      }
      // The clip is gone, is one of sound, or the monitor shows no timeline: the view of the clip before is turned off, where there was one.
      mine.state = 'refused';
      if (wasOn) off();
    });
  }, [bridge, session, wanted, node, told]);

  // The page left: the view is turned off, where this page has it on.
  useEffect(() => () => {
    const last = asked.current;
    asked.current = null;
    if (last === null || last.state !== 'on' || last.bridge !== bridge || last.session !== session) return;
    void callEngine(bridge, 'viewer.fusion', { clip: null }, { expectedSession: session }).catch(() => {});
  }, [bridge, session]);
}
