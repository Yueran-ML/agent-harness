import { AlignCenter, AlignLeft, AlignRight } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Clip, Timeline, TitleStyle } from '../engine';
import { common, setClips } from './common';
import { ColorField, NumberField, SegmentField, TextField } from './fields';
import type { Commit } from './useCommit';

type TitleClip = Clip & { source: Extract<Clip['source'], { kind: 'title' }> };
type ColorClip = Clip & { source: Extract<Clip['source'], { kind: 'color' }> };
/** The longest text of a title that the engine takes. */
export const LONGEST_TEXT = 1000;

/**
 * Titles, as DaVinci Resolve's Text inspector: the text, its size in pixels of the timeline, its
 * colour, where its anchor is (in pixels, kept as a share of the picture) and which side of the
 * text the anchor is on. Shown where every clip chosen on video tracks is a title.
 */
export function ClipTitle({ clips, timeline, commit }: { clips: readonly TitleClip[]; timeline: Timeline; commit: Commit }) {
  const labels = useWords().label.studio.inspector;
  const ids = clips.map(clip => clip.id);
  const style = <K extends keyof TitleStyle>(key: K) => common(clips, clip => clip.source.style[key]);
  const set = (change: (source: TitleClip['source']) => Partial<TitleClip['source']>) => void commit(setClips(ids, clip => clip.source.kind === 'title'
    ? { source: { ...clip.source, ...change(clip.source) } } : null));
  const restyle = (change: Partial<TitleStyle>) => set(source => ({ style: { ...source.style, ...change } }));
  return <div className="inspector-panel" data-panel="title">
    <TextField name="text" label={labels.text} value={common(clips, clip => clip.source.text)} multiline
      onCommit={text => set(() => ({ text: [...text].slice(0, LONGEST_TEXT).join('') }))} />
    <NumberField name="size" label={labels.size} value={style('size')} fallback={Math.max(1, Math.round(timeline.height * 0.065))} unit="px"
      min={1} max={4096} digits={0} scrub={0.5} slider={{ min: 1, max: 400, step: 1 }} onCommit={size => restyle({ size })} />
    <ColorField name="color" label={labels.color} value={style('color')} onCommit={color => restyle({ color })} />
    <NumberField name="x" label={labels.positionX} value={common(clips, clip => clip.source.style.x * timeline.width)} fallback={timeline.width / 2} unit="px"
      min={-timeline.width * 4} max={timeline.width * 5} digits={1} scrub={1} onCommit={x => restyle({ x: x / timeline.width })} />
    <NumberField name="y" label={labels.positionY} value={common(clips, clip => clip.source.style.y * timeline.height)} fallback={timeline.height / 2} unit="px"
      min={-timeline.height * 4} max={timeline.height * 5} digits={1} scrub={1} onCommit={y => restyle({ y: y / timeline.height })} />
    <SegmentField name="align" label={labels.align} value={style('align')} onCommit={align => restyle({ align })} options={[
      { value: 'left', label: labels.alignLeft, icon: <AlignLeft size={14} /> },
      { value: 'center', label: labels.alignCenter, icon: <AlignCenter size={14} /> },
      { value: 'right', label: labels.alignRight, icon: <AlignRight size={14} /> },
    ]} />
  </div>;
}

/** A solid colour, as DaVinci Resolve's generator of one: its colour. */
export function ClipColor({ clips, commit }: { clips: readonly ColorClip[]; commit: Commit }) {
  const labels = useWords().label.studio.inspector;
  const ids = clips.map(clip => clip.id);
  return <div className="inspector-panel" data-panel="color">
    <ColorField name="color" label={labels.color} value={common(clips, clip => clip.source.color)}
      onCommit={color => void commit(setClips(ids, clip => clip.source.kind === 'color' ? { source: { kind: 'color', color } } : null))} />
  </div>;
}

export const isTitle = (clip: Clip): clip is TitleClip => clip.source.kind === 'title';
export const isColor = (clip: Clip): clip is ColorClip => clip.source.kind === 'color';
