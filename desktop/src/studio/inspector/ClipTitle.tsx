import { useContext, type ReactNode } from 'react';
import { AlignCenter, AlignLeft, AlignRight } from 'lucide-react';
import { useWords } from '../../app/locale';
import {
  DEFAULT_TITLE_PARTS,
  type Clip, type ClipKeyed, type Media, type Timeline, type TitleBackground, type TitlePart, type TitleShadow, type TitleStroke, type TitleStyle,
} from '../engine';
import { CLIP_BOUNDS } from './clipKeys';
import { useClipKeyFields } from './ClipKeyFields';
import { common, setClips } from './common';
import { ColorField, FieldsLocked, NumberField, SegmentField } from './fields';
import { FontField } from './TitleFont';
import { TitleText, type SourceChange, type TitleClip } from './TitleText';
import type { Commit } from './useCommit';

type ColorClip = Clip & { source: Extract<Clip['source'], { kind: 'color' }> };
/** The longest text of a title that the engine takes. */
export { LONGEST_TEXT } from './titleSpans';

/** The style of a title with a part of it given, or taken away where it is undefined: then the style says nothing of it. */
export function withPart(style: TitleStyle, part: TitlePart, value: TitleStroke | TitleShadow | TitleBackground | undefined): TitleStyle {
  const next: Record<string, unknown> = { ...style };
  if (value === undefined) delete next[part];
  else next[part] = value;
  return next as unknown as TitleStyle;
}

/**
 * Titles, as DaVinci Resolve's Text inspector: the text, its font, its size in pixels of the timeline, its colour, where its
 * anchor is (in pixels, kept as a share of the picture) and which side of the text the anchor is on; and its Stroke, Drop
 * Shadow and Background. Shown where every clip chosen on video tracks is a title.
 *
 * The size and the place of the anchor take keys, as the settings of the Video tab do, with the
 * same fields and buttons (`useClipKeyFields`): a field shows its setting as it is at the playhead,
 * the diamond beside it puts a key there or takes away the one there, and a value given once the
 * setting has keys is a key at the playhead. The keys of a title count its own frames. The text,
 * the font, the colour and the side of the anchor are one value on every frame: they are set with
 * the source whole, which keeps the keys of the style as they are. The text and its styled runs are
 * `TitleText`'s.
 *
 * The Stroke, the Drop Shadow and the Background are parts of the style that a title has or not: the switch of each gives it to
 * every title chosen (black, 4 pixels wide; black at half, 4 pixels right and down and blurred by 4; black at 60%, 16 pixels past
 * the text with corners of 8), keeping one that a title has, or takes it away. Their fields show where every title chosen has
 * the part: a colour, which keeps its alpha, and numbers in pixels of the timeline that take keys as the size does.
 */
export function ClipTitle({ clips, timeline, commit, media = [], frame = 0, onSeek = () => {} }: {
  clips: readonly TitleClip[]; timeline: Timeline; commit: Commit; media?: readonly Media[]; frame?: number;
  onSeek?: (frame: number, scrubbing: boolean) => void;
}) {
  const labels = useWords().label.studio.inspector;
  const ids = clips.map(clip => clip.id);
  const { value, set: put, keys } = useClipKeyFields({ clips, timeline, commit, media, frame, onSeek });
  const style = <K extends 'font' | 'color' | 'align'>(key: K) => common(clips, clip => clip.source.style[key]);
  const set = (change: (source: TitleClip['source']) => Partial<TitleClip['source']>) => void commit(setClips(ids, clip => clip.source.kind === 'title'
    ? { source: { ...clip.source, ...change(clip.source) } } : null));
  const restyle = (change: Partial<Pick<TitleStyle, 'font' | 'color' | 'align'>>) => set(source => ({ style: { ...source.style, ...change } }));
  const resource = (change: SourceChange) => void commit(setClips(ids, (clip) => {
    if (clip.source.kind !== 'title') return null;
    const source = change(clip.source);
    return source === null ? null : { source };
  }));
  /** Whether every title chosen has a part of the style, none has, or null where some have. */
  const has = (part: TitlePart) => common(clips, clip => clip.source.style[part] !== undefined);
  const turn = (part: TitlePart, on: boolean) => set(source => ({
    style: withPart(source.style, part, on ? source.style[part] ?? { ...DEFAULT_TITLE_PARTS[part] } : undefined),
  }));
  const colorOf = (part: TitlePart) => common(clips, clip => clip.source.style[part]?.color ?? null);
  const recolor = (part: TitlePart, color: string) => set((source) => {
    const holder = source.style[part];
    return holder === undefined ? {} : { style: withPart(source.style, part, { ...holder, color }) };
  });
  /** A number of a part of the style, in pixels of the timeline, with the three buttons of its keys. */
  const pixels = (path: ClipKeyed, name: string, label: string, slider: { min: number; max: number }) => <>
    <NumberField name={name} label={label} value={value(path, kept => kept)} fallback={CLIP_BOUNDS[path].neutral} unit="px"
      min={CLIP_BOUNDS[path].low} max={CLIP_BOUNDS[path].high} digits={1} scrub={0.25} slider={{ ...slider, step: 0.5 }} onCommit={to => put(path, to)} />
    {keys(path, label)}
  </>;
  return <div className="inspector-panel" data-panel="title">
    <TitleText clips={clips} onSource={resource} />
    <FontField name="font" label={labels.font} value={style('font')} placeholder={labels.defaultFont} onCommit={font => restyle({ font })} />
    <NumberField name="size" label={labels.size} value={value('title.size', kept => kept)} fallback={Math.max(1, Math.round(timeline.height * 0.065))} unit="px"
      min={1} max={4096} digits={0} scrub={0.5} slider={{ min: 1, max: 400, step: 1 }} onCommit={size => put('title.size', size)} />
    {keys('title.size', labels.size)}
    <ColorField name="color" label={labels.color} value={style('color')} onCommit={color => restyle({ color })} />
    <NumberField name="x" label={labels.positionX} value={value('title.x', kept => kept * timeline.width)} fallback={timeline.width / 2} unit="px"
      min={-timeline.width * 4} max={timeline.width * 5} digits={1} scrub={1} onCommit={x => put('title.x', x / timeline.width)} />
    {keys('title.x', labels.positionX)}
    <NumberField name="y" label={labels.positionY} value={value('title.y', kept => kept * timeline.height)} fallback={timeline.height / 2} unit="px"
      min={-timeline.height * 4} max={timeline.height * 5} digits={1} scrub={1} onCommit={y => put('title.y', y / timeline.height)} />
    {keys('title.y', labels.positionY)}
    <SegmentField name="align" label={labels.align} value={style('align')} onCommit={align => restyle({ align })} options={[
      { value: 'left', label: labels.alignLeft, icon: <AlignLeft size={14} /> },
      { value: 'center', label: labels.alignCenter, icon: <AlignCenter size={14} /> },
      { value: 'right', label: labels.alignRight, icon: <AlignRight size={14} /> },
    ]} />
    <TitlePartSection part="stroke" label={labels.stroke} on={has('stroke')} onSwitch={on => turn('stroke', on)}>
      <ColorField name="stroke-color" label={labels.strokeColor} value={colorOf('stroke')} onCommit={color => recolor('stroke', color)} />
      {pixels('title.stroke.width', 'stroke-width', labels.strokeWidth, { min: 0, max: 32 })}
    </TitlePartSection>
    <TitlePartSection part="shadow" label={labels.shadow} on={has('shadow')} onSwitch={on => turn('shadow', on)}>
      <ColorField name="shadow-color" label={labels.shadowColor} value={colorOf('shadow')} onCommit={color => recolor('shadow', color)} />
      {pixels('title.shadow.x', 'shadow-x', labels.shadowX, { min: -64, max: 64 })}
      {pixels('title.shadow.y', 'shadow-y', labels.shadowY, { min: -64, max: 64 })}
      {pixels('title.shadow.blur', 'shadow-blur', labels.shadowBlur, { min: 0, max: 64 })}
    </TitlePartSection>
    <TitlePartSection part="background" label={labels.background} on={has('background')} onSwitch={on => turn('background', on)}>
      <ColorField name="background-color" label={labels.backgroundColor} value={colorOf('background')} onCommit={color => recolor('background', color)} />
      {pixels('title.background.padding', 'background-padding', labels.backgroundPadding, { min: 0, max: 128 })}
      {pixels('title.background.radius', 'background-radius', labels.backgroundRadius, { min: 0, max: 128 })}
    </TitlePartSection>
  </div>;
}

/**
 * A part of the style of the titles chosen that they may have or not: its heading with the switch that gives it or takes it
 * away (mixed where some have it), and its fields where every one has it.
 */
function TitlePartSection({ part, label, on, onSwitch, children }: {
  part: TitlePart; label: string; on: boolean | null; onSwitch: (on: boolean) => void; children: ReactNode;
}) {
  const locked = useContext(FieldsLocked);
  return <div className="inspector-title-part" data-section={`title-${part}`}>
    <div className="inspector-section-row inspector-flag">
      <h3 className="inspector-section">{label}</h3>
      <input type="checkbox" role="switch" aria-label={label} title={label} checked={on === true} disabled={locked} data-test={`title-${part}`}
        ref={(node) => { if (node) node.indeterminate = on === null; }} onChange={event => onSwitch(event.target.checked)} />
    </div>
    {on === true && children}
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
