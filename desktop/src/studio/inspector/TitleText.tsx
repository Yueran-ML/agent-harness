import { useContext, useEffect, useId, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { Bold, Italic, RemoveFormatting } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Clip } from '../engine';
import { common } from './common';
import { FieldsLocked, MIXED, useHeld } from './fields';
import { FontInput } from './TitleFont';
import {
  clipped, lengthOf, LEAST_SCALE, lookOver, MOST_SCALE, runsOf, scalarAt, shifted, styled, withSpans, type SpanLook, type SpanPatch,
} from './titleSpans';

export type TitleSource = Extract<Clip['source'], { kind: 'title' }>;
export type TitleClip = Clip & { source: TitleSource };
/** A change of the source of a title, worked out from it as it is when it is sent: null leaves it as it is. */
export type SourceChange = (source: TitleSource) => TitleSource | null;

/** The characters chosen in the text, in Unicode scalar values: from `from` to the one before `to`. */
interface Chosen { from: number; to: number }

/** A share of the size as the bar writes it: up to two decimals, and no more than it needs. */
const scaleText = (scale: number) => String(Number(scale.toFixed(2)));

/** A share of the size typed into the bar, kept within those that a run may have and to two decimals; null for none. */
function parseScale(text: string): number | null {
  const trimmed = text.trim().replace(',', '.');
  if (trimmed === '') return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? Number(Math.min(MOST_SCALE, Math.max(LEAST_SCALE, value)).toFixed(2)) : null;
}

/**
 * How a part of the text is drawn in the line under the field that shows its runs: in its colour, weight, slant and family, and
 * at its share of the size held between half and twice that of the line, so that the line stays one.
 */
function cssOf(look: SpanLook): CSSProperties {
  return {
    color: look.color,
    fontWeight: look.bold ? 700 : undefined,
    fontStyle: look.italic ? 'italic' : undefined,
    fontFamily: look.font ? `"${look.font.replace(/["\\]/g, '')}"` : undefined,
    fontSize: look.scale === undefined ? undefined : `${Math.min(2, Math.max(0.5, look.scale))}em`,
  };
}

/**
 * The text of the titles chosen (Reference Manual 20, the Text title), and a bar over it that styles the characters chosen in
 * it: bold, italic, a colour, a share of the size and a family, and Clear, which takes all of it away. What the bar does is
 * kept in the runs of the style (`TitleSpan`, `titleSpans.ts`): it changes those over the characters chosen, cutting a run
 * that goes past an end of them, and sends the source whole with them.
 *
 * - The text is sent when Enter is pressed (Shift+Enter starts a new line) or the field is left, as the other fields of text
 *   are, cut to the longest that the engine takes; the runs of each title go with it, moved and cut to stay on the characters
 *   that they were on (`shifted`), since the engine refuses a text that ends before one of them.
 * - The bar works on what the field has: text typed and not yet sent goes in the same change as what the bar does. Its
 *   buttons do not take the focus from the field, so that the selection stays where it is seen. Where several titles are
 *   chosen, it works where they have the same text, on the runs of each.
 * - A change that would leave a title more runs than the engine takes is not sent, and the bar says why.
 * - Where the titles have runs, a line under the field shows the text drawn as they say.
 */
export function TitleText({ clips, onSource }: {
  clips: readonly TitleClip[];
  /** Change the source of every title chosen, each as it is when the change is sent. */
  onSource: (change: SourceChange) => void;
}) {
  const words = useWords();
  const labels = words.label.studio.inspector;
  const said = words.hint.studio.inspector;
  const locked = useContext(FieldsLocked);
  const id = useId();
  const value = common(clips, clip => clip.source.text);
  const [draft, setDraft] = useState<string | null>(null);
  const [held, hold] = useHeld(value);
  const [chosen, setChosen] = useState<Chosen | null>(null);
  const [full, setFull] = useState(false);
  const [scaleDraft, setScaleDraft] = useState<string | null>(null);
  const [liveColour, setLiveColour] = useState<string | null>(null);
  const area = useRef<HTMLTextAreaElement>(null);
  const swatch = useRef<HTMLInputElement>(null);
  /** The characters chosen as they were last read, so that a selection told again changes nothing. */
  const read = useRef<Chosen | null>(null);
  const which = clips.map(clip => clip.id).join(' ');
  // Other titles chosen: nothing of the text of those before is chosen.
  useEffect(() => { read.current = null; setChosen(null); setFull(false); }, [which]);

  const shown = draft ?? held?.value ?? value ?? '';
  const length = lengthOf(shown);
  /** The runs of the titles as they would be on the text that the field shows, or null where the titles differ. */
  const spans = common(clips, clip => clip.source.style.spans ?? []);
  const current = spans === null || value === null ? null : shown === value ? spans : shifted(spans, value, shown);
  const range = chosen !== null && Math.min(chosen.to, length) > chosen.from ? { from: chosen.from, to: Math.min(chosen.to, length) } : null;
  const ready = !locked && value !== null && range !== null && (draft === null || draft.trim() !== '');
  const over = ready && range !== null && current !== null ? lookOver(current, range.from, range.to) : null;
  const base = common(clips, clip => clip.source.style.color) ?? '#ffffffff';
  const colour = over !== null && typeof over.color === 'string' ? over.color : base;

  /**
   * Read the characters chosen from the field, as it tells of them in units of UTF-16. A selection that is as it was changes
   * nothing: the field tells of one more than once (`select` comes again a moment after a script sets one).
   */
  const measure = () => {
    const element = area.current;
    if (!element) return;
    const text = element.value;
    const from = scalarAt(text, Math.min(element.selectionStart, element.selectionEnd));
    const to = scalarAt(text, Math.max(element.selectionStart, element.selectionEnd));
    const next = from < to ? { from, to } : null;
    const before = read.current;
    if (next === null ? before === null : before !== null && before.from === next.from && before.to === next.to) return;
    read.current = next;
    setChosen(next);
    setFull(false);
  };
  useEffect(() => {
    // A selection made by a script, or by the browser itself, is told only by `select`.
    const element = area.current;
    if (!element) return undefined;
    element.addEventListener('select', measure);
    return () => element.removeEventListener('select', measure);
  }, []);

  const send = () => {
    if (draft === null) return;
    setDraft(null);
    if (draft.trim() === '' || draft === value) return;
    hold(draft);
    const text = clipped(draft);
    onSource(source => (source.text === text ? null
      : { ...source, text, style: withSpans(source.style, shifted(source.style.spans ?? [], source.text, text)) }));
  };
  const key = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); send(); }
    else if (event.key === 'Escape' && draft !== null) { event.preventDefault(); event.stopPropagation(); setDraft(null); }
  };

  /** Make a change to what the characters chosen say, in every title chosen, with the text typed and not yet sent. */
  const apply = (patch: SpanPatch) => {
    if (!ready || range === null) return;
    const { from, to } = range;
    const typed = draft !== null && draft !== value ? clipped(draft) : null;
    const after = (source: TitleSource) => {
      const text = typed ?? source.text;
      const before = text === source.text ? source.style.spans ?? [] : shifted(source.style.spans ?? [], source.text, text);
      const made = styled(before, from, to, patch, lengthOf(text));
      return made === null ? null : { ...source, text, style: withSpans(source.style, made) };
    };
    if (clips.some(clip => after(clip.source) === null)) { setFull(true); return; }
    if (draft !== null) {
      setDraft(null);
      if (typed !== null) hold(draft);
    }
    onSource(after);
  };
  const now = useRef({ apply, alpha: 'ff' });
  now.current = { apply, alpha: colour.slice(7, 9) || 'ff' };
  useEffect(() => {
    // The colour is sent as the picker is closed, with the alpha that the characters had.
    const element = swatch.current;
    if (!element) return undefined;
    const changed = () => {
      setLiveColour(null);
      const { apply: put, alpha } = now.current;
      put({ color: `${element.value.toLowerCase()}${alpha}` });
    };
    element.addEventListener('change', changed);
    return () => element.removeEventListener('change', changed);
  }, []);

  const sendScale = () => {
    if (scaleDraft === null) return;
    setScaleDraft(null);
    const scale = parseScale(scaleDraft);
    if (scale === null || over === null || (over.scale !== null && scale === (over.scale ?? 1))) return;
    apply({ scale: scale === 1 ? null : scale });
  };
  const scaleKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') { event.preventDefault(); sendScale(); }
    else if (event.key === 'Escape' && scaleDraft !== null) { event.preventDefault(); event.stopPropagation(); setScaleDraft(null); }
  };
  /** A button of the bar: pressed, it keeps the focus in the field, so that its selection stays shown. */
  const keep = (event: { preventDefault: () => void }) => event.preventDefault();
  const parts = current !== null && current.length > 0 ? runsOf(shown, current) : null;

  return <div className="inspector-title-text" data-section="title-text">
    <div className="inspector-row inspector-row-wide" data-field="text" data-mixed={value === null || undefined}>
      <label htmlFor={id} className="inspector-label">{labels.text}</label>
      <div className="title-span-bar" role="toolbar" aria-label={labels.textStyle} title={ready ? undefined : said.spans} data-test="title-span-bar">
        <button type="button" className="icon-btn" aria-label={labels.bold} title={labels.bold} aria-pressed={over?.bold === true} disabled={!ready}
          data-test="span-bold" onMouseDown={keep} onClick={() => apply({ bold: over?.bold ? null : true })}><Bold size={13} aria-hidden="true" /></button>
        <button type="button" className="icon-btn" aria-label={labels.italic} title={labels.italic} aria-pressed={over?.italic === true} disabled={!ready}
          data-test="span-italic" onMouseDown={keep} onClick={() => apply({ italic: over?.italic ? null : true })}><Italic size={13} aria-hidden="true" /></button>
        <input ref={swatch} className="inspector-color title-span-color" type="color" aria-label={labels.textColor} title={labels.textColor}
          value={liveColour ?? colour.slice(0, 7)} disabled={!ready} data-test="span-color"
          onChange={event => { if (event.nativeEvent.type !== 'change') setLiveColour(event.target.value); }}
          onKeyDown={event => { if (event.key === 'Escape' && liveColour !== null) { event.stopPropagation(); setLiveColour(null); } }} />
        <span className="title-span-scale">
          <input className="field" type="text" inputMode="decimal" autoComplete="off" spellCheck={false}
            aria-label={labels.sizeFactor} title={labels.sizeFactor} disabled={!ready} data-test="span-scale"
            value={scaleDraft ?? (over === null || over.scale === null ? '' : scaleText(over.scale ?? 1))}
            placeholder={over !== null && over.scale === null ? MIXED : undefined}
            onChange={event => setScaleDraft(event.target.value)} onKeyDown={scaleKey} onBlur={sendScale} />
          <span className="inspector-unit" aria-hidden="true">×</span>
        </span>
        <FontInput value={over === null ? '' : over.font === null ? null : over.font ?? ''} label={labels.font} placeholder={labels.defaultFont}
          disabled={!ready} className="field title-span-font" test="span-font" onCommit={family => apply({ font: family === '' ? null : family })} />
        <button type="button" className="icon-btn" aria-label={labels.clearStyle} title={labels.clearStyle} disabled={!ready} data-test="span-clear"
          onMouseDown={keep} onClick={() => apply({ color: null, scale: null, bold: null, italic: null, font: null })}>
          <RemoveFormatting size={13} aria-hidden="true" /></button>
      </div>
      <textarea ref={area} id={id} className="field inspector-text" rows={3} spellCheck={false} disabled={locked}
        value={shown} placeholder={value === null ? MIXED : undefined}
        onChange={(event) => { setDraft(event.target.value); measure(); }} onKeyDown={key} onBlur={send}
        onSelect={measure} onKeyUp={measure} onMouseUp={measure} />
    </div>
    {full && <p className="caption title-span-note" role="status" data-test="title-span-full">{said.spansFull}</p>}
    {parts !== null && <p className="title-text-preview" aria-hidden="true" data-test="title-text-preview">
      {parts.map((part, at) => <span key={at} style={cssOf(part.look)}>{part.text}</span>)}
    </p>}
  </div>;
}
