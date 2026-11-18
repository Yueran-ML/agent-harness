import { useContext, useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { FieldsLocked, MIXED, useHeld } from './fields';

/**
 * The families offered for the font of a title where the page cannot read those of the system: some that Windows, macOS and
 * Linux come with, for Latin, Chinese and Japanese. Any other may be typed; the engine draws a family that the system has not
 * got in the one that it chooses, as it does for none.
 */
export const TITLE_FONTS: readonly string[] = [
  'Arial', 'Arial Black', 'Bahnschrift', 'Calibri', 'Cambria', 'Candara', 'Comic Sans MS', 'Consolas', 'Constantia', 'Corbel',
  'Courier New', 'Franklin Gothic Medium', 'Gabriola', 'Georgia', 'Helvetica', 'Helvetica Neue', 'Impact', 'Segoe Print',
  'Segoe Script', 'Segoe UI', 'Tahoma', 'Times New Roman', 'Trebuchet MS', 'Verdana',
  'Microsoft YaHei', 'Microsoft JhengHei', 'DengXian', 'SimHei', 'SimSun', 'KaiTi', 'FangSong', 'PingFang SC', 'Noto Sans SC',
  'Noto Serif SC', 'Source Han Sans SC', 'Yu Gothic', 'Yu Mincho', 'Meiryo', 'MS Gothic', 'MS Mincho', 'Hiragino Sans',
  'Noto Sans JP', 'Noto Sans', 'Noto Serif', 'DejaVu Sans', 'Liberation Sans', 'Liberation Serif',
];

/** The families of the fonts of the system, once they were read. */
let read: readonly string[] | null = null;
let asking: Promise<readonly string[]> | null = null;

/**
 * The families of the fonts of the system, where the page may read them (Local Font Access, which Chromium lets a page use on a
 * gesture of the user), in the order of their names; else, or where the system gives none, `TITLE_FONTS`. Read once: a
 * refusal is asked again at the next gesture.
 */
export function systemFonts(): Promise<readonly string[]> {
  if (read) return Promise.resolve(read);
  if (asking) return asking;
  const query = (globalThis as { queryLocalFonts?: () => Promise<readonly { family: string }[]> }).queryLocalFonts;
  if (typeof query !== 'function') return Promise.resolve(TITLE_FONTS);
  asking = query.call(globalThis).then((fonts) => {
    const families = [...new Set(fonts.map(font => font.family).filter(family => family !== ''))].sort((a, b) => a.localeCompare(b));
    if (families.length > 0) read = families;
    return families.length > 0 ? families : TITLE_FONTS;
  }, () => TITLE_FONTS).finally(() => { asking = null; });
  return asking;
}

/** The families to offer, and what reads those of the system: to be called on a gesture of the user, as a press. */
export function useFontFamilies(): { families: readonly string[]; load: () => void } {
  const [families, setFamilies] = useState<readonly string[]>(() => read ?? TITLE_FONTS);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => { live.current = false; };
  }, []);
  const load = () => {
    if (read) { setFamilies(read); return; }
    void systemFonts().then((found) => { if (live.current) setFamilies(found); });
  };
  return { families, load };
}

/**
 * The family of a font, typed or chosen from those offered (`useFontFamilies`): sent when Enter is pressed or the box is left,
 * where it changed; Escape puts back what was there. Empty, the one that the style says, or that the engine chooses.
 */
export function FontInput({ id, value, label, placeholder, disabled = false, className = 'field inspector-text', test, onCommit }: {
  id?: string;
  /** The family, '' for none, or null where what it sets differs. */
  value: string | null;
  /** Its name, where no label of its row names it. */
  label?: string;
  /** What it shows where it is empty. */
  placeholder: string;
  disabled?: boolean; className?: string; test?: string;
  onCommit: (family: string) => void;
}) {
  const locked = useContext(FieldsLocked);
  const list = useId();
  const { families, load } = useFontFamilies();
  const [draft, setDraft] = useState<string | null>(null);
  const [held, hold] = useHeld(value);
  const send = () => {
    if (draft === null) return;
    setDraft(null);
    const family = draft.trim();
    if (value !== null && family === value) return;
    hold(family);
    onCommit(family);
  };
  const key = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === 'Enter') { event.preventDefault(); send(); }
    else if (event.key === 'Escape' && draft !== null) { event.preventDefault(); event.stopPropagation(); setDraft(null); }
  };
  const shown = held?.value ?? value;
  return <>
    <input id={id} className={className} type="text" autoComplete="off" spellCheck={false} list={list} aria-label={label} title={label}
      value={draft ?? shown ?? ''} placeholder={shown === null ? MIXED : placeholder} disabled={locked || disabled} data-test={test}
      onPointerDown={load} onChange={event => setDraft(event.target.value)} onKeyDown={key} onBlur={send} />
    <datalist id={list}>{families.map(family => <option key={family} value={family} />)}</datalist>
  </>;
}

/** The font of the titles chosen, as a row of the Inspector: see `FontInput`. */
export function FontField({ name, label, value, placeholder, onCommit }: {
  name: string; label: string; value: string | null; placeholder: string; onCommit: (family: string) => void;
}) {
  const id = useId();
  return <div className="inspector-row inspector-font" data-field={name} data-mixed={value === null || undefined}>
    <label htmlFor={id} className="inspector-label">{label}</label>
    <span className="inspector-value"><FontInput id={id} value={value} placeholder={placeholder} onCommit={onCommit} /></span>
  </div>;
}
