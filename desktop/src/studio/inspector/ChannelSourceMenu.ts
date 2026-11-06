import { useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { MenuItem } from '../color/ColorMenu';
import { callEngine, type ChannelSource, type Media, type Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { channelSources } from '../media/audioLayout';
import { clipLocations } from '../timeline/selection';

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
    if (clip.channels == null) return [{ id: 'automatic-channel-source',
      label: t('Automatic mix: configure channel sources in File first.'), disabled: true }];
    if (!media?.facts.audio?.streams?.length) return [{ id: 'missing-channel-sources', label: t('Audio stream information is unavailable.'), disabled: true }];
    const change = async (index: number, source: ChannelSource | null) => {
      if (busy.current || !live.current) return;
      const current = now.current;
      if (current.session !== captured.session || current.timeline?.id !== captured.timeline?.id || current.revision !== captured.revision) {
        setProblem(t('The clip changed. Reopen the channel source menu.')); return;
      }
      const latest = current.timeline && clipLocations(current.timeline).get(context.clip);
      if (!latest || latest.track.locked || latest.clip.channels == null || !latest.clip.channels[index]) return;
      const previous = latest.clip.channels[index].source;
      if (JSON.stringify(channelSources(previous)) === JSON.stringify(source ? [source] : [])) return;
      const channels = latest.clip.channels.map((channel, at) => at === index ? { ...channel, source } : channel);
      busy.current = true; setPending(true); setProblem(null);
      try {
        await callEngine(current.bridge, 'transaction.apply', { revision: captured.revision,
          ops: [{ op: 'set_clip', timeline: current.timeline!.id, id: clip.id, channels }],
        }, { expectedSession: captured.session });
      } catch (error) {
        if (live.current && now.current.session === captured.session) setProblem(`${t('Could not change channel source.')} ${error instanceof Error ? error.message : String(error)}`);
      } finally { busy.current = false; if (live.current) setPending(false); }
    };
    const choices = (index: number): MenuItem[] => {
      const map = clip.channels?.[index];
      if (!map) return [];
      const selected = channelSources(map.source);
      const disabled = track.locked || pending;
      return [{ id: `channel-${index}-none`, label: t('None'), checked: selected.length === 0, disabled, run: () => void change(index, null) },
        ...media.facts.audio!.streams!.flatMap(stream => Array.from({ length: stream.channels }, (_, channel) => ({
          id: `channel-${index}-source-${stream.index}-${channel}`,
          label: `${t('Stream')} ${stream.index} · ${t('Channel')} ${channel + 1}`,
          checked: selected.length === 1 && selected[0].stream === stream.index && selected[0].channel === channel,
          disabled, run: () => void change(index, { stream: stream.index, channel }),
        })))];
    };
    if (context.channel !== undefined) return choices(context.channel);
    return [{ id: 'audio-channel', label: t('Audio Channel'), items: clip.channels.map((_, index) => ({
      id: `audio-channel-${index}`, label: `${t('Channel')} ${index + 1}`, items: choices(index),
    })) }];
  };
  return { items, problem, clear: () => setProblem(null) };
}
