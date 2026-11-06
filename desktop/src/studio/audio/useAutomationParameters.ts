import type { EngineBridge } from '../../bridge';
import { callEngine, type LanesOf, type Timeline } from '../engine';
import { useAudioQuery } from './useAudioQuery';

interface Context {
  bridge: Pick<EngineBridge, 'call'>;
  session: number;
  revision: number;
  timeline: Timeline;
}

/** The server derives both schema and current values from the owner's strip/mute.
 * Include values, not only structure: lane drawing must not reuse a stale fixed value.
 */
export function useAutomationParameters(context: Context, of: LanesOf | null, enabled = true) {
  const { bridge, session, revision, timeline } = context;
  const owner = of && ('track' in of
    ? timeline.tracks.find(row => row.kind === 'audio' && row.id === of.track)
    : 'bus' in of ? timeline.buses?.find(row => row.id === of.bus)
      : timeline.vcas?.find(row => row.id === of.vca));
  const settings = owner && of && ('vca' in of
    ? { fader_db: 'fader_db' in owner ? owner.fader_db : undefined, mute: 'mute' in owner ? owner.mute : undefined }
    : { strip: 'strip' in owner ? owner.strip : undefined, mute: 'mute' in owner ? owner.mute : undefined });
  const { data, problem } = useAudioQuery(bridge,
    { kind: 'automation.parameters', session, timeline: timeline.id, of, settings }, revision,
    enabled && of !== null, () => callEngine(bridge, 'automation.parameters',
      { timeline: timeline.id, of: of! }, { expectedSession: session }));
  return { parameters: data?.parameters ?? [], problem };
}
