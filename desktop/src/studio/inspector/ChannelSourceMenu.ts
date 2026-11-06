import { useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { MenuItem } from '../color/ColorMenu';
import { callEngine, type ChannelMap, type ChannelSource, type Clip, type Media, type Timeline, type Track } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { automaticChannels, channelSources } from '../media/audioLayout';
import { clipLocations } from '../timeline/selection';

/** What a choice of the menu was made from: the clip's source and channels, and its track's layout. */
const menuSnapshot = (clip: Clip, track: Track) => [clip.source, clip.channels ?? null, automaticChannels(track)];
export interface ChannelContext { channel?: number; sourcesOnly?: boolean }
export interface AudioClipContext extends ChannelContext { clip: string; x: number; y: number }

/** Both pages edit the clicked channel through the same revision-checked transaction. */
export function useChannelSourceMenu(props: {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number;
  timeline: Timeline | null; media: readonly Media[];
}) {
  const t = useFairlightText();
  const now = useRef(props); now.current = props;
  const live = useRef(true), busy = useRef(false);
  const [pending, setPending] = useState(false), [problem, setProblem] = useState<string | null>(null);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  useEffect(() => setProblem(null), [props.session, props.timeline?.id]);
  const items = (context: AudioClipContext): MenuItem[] => {
    const captured = now.current;
    const located = captured.timeline && clipLocations(captured.timeline).get(context.clip);
    if (!located || located.track.kind !== 'audio' || located.clip.source.kind !== 'media') return [];
    const { clip, track } = located;
    const sourceId = clip.source.kind === 'media' ? clip.source.media : '';
    const media = captured.media.find(item => item.id === sourceId);
    const automatic = automaticChannels(track), maps = clip.channels ?? automatic;
    if (!media?.facts.audio?.streams?.length) return [{ id: 'missing-channel-sources', label: t('Audio stream information is unavailable.'), disabled: true }];
    const change = async (index: number, source: ChannelSource | null, remix?: ChannelMap['remix']) => {
      if (busy.current || !live.current) return;
      const current = now.current;
      const latest = current.timeline && clipLocations(current.timeline).get(context.clip);
      // An edit elsewhere since the menu opened does not refuse the choice: only one of this clip
      // (its source or channels) or of what its automatic layout comes from (its track's format).
      const same = latest && latest.track.kind === 'audio' && latest.clip.source.kind === 'media'
        && JSON.stringify(menuSnapshot(latest.clip, latest.track)) === JSON.stringify(menuSnapshot(clip, track));
      if (current.session !== captured.session || current.timeline?.id !== captured.timeline?.id || !same) {
        setProblem(t('The clip changed. Reopen the channel source menu.')); return;
      }
      if (!latest || latest.track.locked) return;
      const before = latest.clip.channels ?? automaticChannels(latest.track), previous = before[index];
      if (!previous) return;
      if (JSON.stringify([channelSources(previous.source), previous.remix ?? null]) === JSON.stringify([source ? [source] : [], remix ?? null])) return;
      const channels = before.map((channel, at) => at === index ? { ...channel, source, remix } : channel);
      busy.current = true; setPending(true); setProblem(null);
      try {
        await callEngine(current.bridge, 'transaction.apply', { revision: current.revision,
          ops: [{ op: 'set_clip', timeline: current.timeline!.id, id: clip.id, channels }],
        }, { expectedSession: captured.session });
      } catch (error) {
        if (live.current && now.current.session === captured.session) setProblem(`${t('Could not change channel source.')} ${error instanceof Error ? error.message : String(error)}`);
      } finally { busy.current = false; if (live.current) setPending(false); }
    };
    const choices = (index: number): MenuItem[] => {
      const map = maps[index];
      if (!map) return [];
      const selected = channelSources(map.source);
      const disabled = track.locked || pending;
      return [{ id: `channel-${index}-none`, label: t('None'), checked: selected.length === 0 && !map.remix, disabled, run: () => void change(index, null) },
        ...media.facts.audio!.streams!.flatMap(stream => Array.from({ length: stream.channels }, (_, channel) => ({
          id: `channel-${index}-source-${stream.index}-${channel}`,
          label: `${t('Stream')} ${stream.index} · ${t('Channel')} ${channel + 1}`,
          checked: selected.length === 1 && selected[0].stream === stream.index && selected[0].channel === channel,
          disabled, run: () => void change(index, { stream: stream.index, channel }),
        }))), { id: `channel-${index}-automatic`, label: t('Automatic mix'),
          checked: JSON.stringify(map.remix) === JSON.stringify(automatic[index]?.remix), disabled,
          run: () => void change(index, null, automatic[index]?.remix) }];
    };
    if (context.channel !== undefined) return choices(context.channel);
    return [{ id: 'audio-channel', label: t('Audio Channel'), items: maps.map((_, index) => ({
      id: `audio-channel-${index}`, label: `${t('Channel')} ${index + 1}`, items: choices(index),
    })) }];
  };
  return { items, problem, clear: () => setProblem(null) };
}
