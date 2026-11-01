import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import { callEngine } from '../engine';
import { editingAllowed, inStudio } from '../keys';
import { autoColor, curvesChanged, primariesChanged } from './corrector';
import { CurvesPalette, type CurveMode } from './Curves';
import { PrimariesPalette, type PrimariesMode } from './Primaries';
import { useNodeEdit, type EditedNode, type NodeEdit } from './useNodeEdit';
import './palettes.css';

/**
 * The palettes of the Color page, in DaVinci Resolve's order (Reference Manual 20, pp. 2975–2977):
 * the left ones (of which this studio has Primaries), the center ones (Curves, Qualifier, Window,
 * Tracker, Blur, Key), and at the right the Keyframe Editor or the scopes. Palettes this studio
 * does not have are not shown, not even greyed (the interaction reference, §1.1).
 */
export const LEFT_PALETTES = ['primaries'] as const;
export const CENTER_PALETTES = ['curves', 'qualifier', 'window', 'tracker', 'blur', 'key'] as const;
export const RIGHT_PANELS = ['keyframes', 'scopes'] as const;
export type LeftPalette = typeof LEFT_PALETTES[number];
export type CenterPalette = typeof CENTER_PALETTES[number];
export type RightPanel = typeof RIGHT_PANELS[number];
export type Palette = LeftPalette | CenterPalette | RightPanel;

/** What the palettes that have controls need: the current node's editing, and the state of their modes. */
interface Filled {
  edit: NodeEdit;
  primaries: PrimariesMode; onPrimaries: (mode: PrimariesMode) => void;
  curves: CurveMode; onCurves: (mode: CurveMode) => void;
  onAuto: () => void; status: string | null;
  /** The palettes in which the current node is adjusted, which show an orange dot (p. 2976). */
  changed: ReadonlySet<Palette>;
  /** The docked video scopes (T19), shown where the Scopes panel is chosen. */
  scopes?: ReactNode;
  windows?: ReactNode;
}

/** One group of palettes: its buttons along its top, and the palette chosen beneath them. */
function PaletteGroup<T extends Palette>({ name, palettes, chosen, onChoose, side, filled }: {
  name: string; palettes: readonly T[]; chosen: T; onChoose: (palette: T) => void; side: 'left' | 'center' | 'right'; filled: Filled;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage.palette;
  // The buttons as a keyboard knows them: one takes the Tab key, the arrows go from one to the other.
  const keys = (event: KeyboardEvent) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    event.stopPropagation();
    const at = palettes.indexOf(chosen);
    const next = palettes[(at + (event.key === 'ArrowRight' ? 1 : palettes.length - 1)) % palettes.length];
    onChoose(next);
    event.currentTarget.querySelector<HTMLElement>(`[data-palette="${next}"]`)?.focus();
  };
  const { edit } = filled;
  const body = (() => {
    if (chosen === 'window' && filled.windows !== undefined) return filled.windows;
    if (chosen === 'scopes' && filled.scopes !== undefined) return filled.scopes;
    if (chosen !== 'primaries' && chosen !== 'curves') return <p className="caption color-empty">{words.hint.studio.colorPage.palette}</p>;
    if (!edit.enabled) return <p className="caption color-empty" data-test="no-corrector">{words.hint.studio.colorPage.noCorrector}</p>;
    return chosen === 'primaries'
      ? <PrimariesPalette key={edit.interaction} edit={edit} mode={filled.primaries} onMode={filled.onPrimaries} onAuto={filled.onAuto} status={filled.status} />
      : <CurvesPalette key={edit.interaction} edit={edit} mode={filled.curves} onMode={filled.onCurves} />;
  })();
  return <section className="color-palette-group" data-side={side} aria-label={name} data-test={`palettes-${side}`}>
    <div className="color-palette-tabs" role="tablist" aria-label={name} onKeyDown={keys}>
      {palettes.map(each => <button key={each} type="button" role="tab" aria-selected={each === chosen} tabIndex={each === chosen ? 0 : -1}
        data-palette={each} data-changed={filled.changed.has(each) || undefined} onClick={() => onChoose(each)}>
        {labels[each]}
        {filled.changed.has(each) && <span className="color-palette-dot" role="img" aria-label={words.label.studio.colorPage.changed}
          title={words.label.studio.colorPage.changed} data-test="changed-dot" />}
      </button>)}
    </div>
    <div className="color-palette" role="tabpanel" aria-label={labels[chosen]} data-palette-shown={chosen}
      onPointerDownCapture={edit.touch} onChangeCapture={edit.touch} onKeyDownCapture={edit.touch}>{body}</div>
  </section>;
}

/**
 * Auto Color, the A button of the Primaries palette (Color › Auto Color, Alt+Shift+C; pp. 3045–3046):
 * the figures of the frame at the playhead, as the timeline monitor shows it, balance the blacks and
 * the whites of the current node, written into its Lift and Gain where they can be seen and changed
 * on, as one step to undo. Where the frame is too flat to tell by, it says so and does nothing.
 */
function useAutoColor(edited: EditedNode | null, edit: NodeEdit) {
  const said = useWords().hint.studio.colorPage;
  const [status, setStatus] = useState<string | null>(null);
  const now = useRef({ edited, edit, said });
  now.current = { edited, edit, said };
  const request = useRef(0);
  useEffect(() => { request.current += 1; setStatus(null); }, [edit.interaction, edited?.revision, edited?.timelineFrame]);
  const run = useCallback(() => {
    const { edited: context, edit: editing } = now.current;
    const generation = ++request.current;
    if (context === null || !editing.enabled || editing.pending()) return;
    const local = editing.localVersion();
    const valid = () => generation === request.current && editing.isCurrent() && local === editing.localVersion()
      && now.current.edited?.revision === context.revision && now.current.edited?.timelineFrame === context.timelineFrame;
    const channels = ['r', 'g', 'b'] as const;
    const original = {
      lift: channels.map(ch => editing.value(`primaries.lift.${ch}`)) as [number, number, number],
      gain: channels.map(ch => editing.value(`primaries.gain.${ch}`)) as [number, number, number],
    };
    setStatus(null);
    void callEngine(context.bridge, 'scopes.statistics', { view: 'timeline' },
      context.session === undefined ? undefined : { expectedSession: context.session }).then((stats) => {
      if (!valid() || (context.timelineFrame !== undefined && stats.frame !== undefined && stats.frame !== context.timelineFrame)) return;
      const result = autoColor(stats, original);
      if (result === null) { setStatus(now.current.said.autoFlat); return; }
      editing.commit(channels.flatMap((ch, i) => [[`primaries.lift.${ch}`, result.lift[i]], [`primaries.gain.${ch}`, result.gain[i]]] as const));
    }).catch(failure => { if (valid()) context.onFailure(failure); });
  }, []);
  return { run, status };
}

/**
 * The palette area at the foot of the page. Where the page is narrow (DaVinci Resolve merges them
 * below 1920×1080, p. 2976; here by the width of the page), the left and center palettes are one
 * group, in one row of buttons, which shows the one of the two chosen last.
 *
 * Primaries and Curves edit the current node of the grade shown (`edited`), where it is a
 * corrector; their buttons carry an orange dot where that node is adjusted in them. Alt+Z switches
 * the Primaries palette between its Wheels and Log modes (p. 3068), and Alt+Shift+C is Auto Color.
 */
export function Palettes({ merged, left, center, last, right, onLeft, onCenter, onRight, edited = null, scopes, windows }: {
  merged: boolean;
  left: LeftPalette; center: CenterPalette;
  /** Which of the two groups was chosen from last. */
  last: 'left' | 'center';
  right: RightPanel;
  onLeft: (palette: LeftPalette) => void; onCenter: (palette: CenterPalette) => void; onRight: (panel: RightPanel) => void;
  /** The current node and where it is, or null where there is none. */
  edited?: EditedNode | null;
  /** The docked video scopes, for the Scopes panel. */
  scopes?: ReactNode;
  windows?: ReactNode;
}) {
  const labels = useWords().label.studio.colorPage;
  const both: readonly (LeftPalette | CenterPalette)[] = [...LEFT_PALETTES, ...CENTER_PALETTES];
  const isLeft = (palette: Palette): palette is LeftPalette => (LEFT_PALETTES as readonly Palette[]).includes(palette);
  const edit = useNodeEdit(edited);
  const [primaries, setPrimaries] = useState<PrimariesMode>('wheels');
  const [curves, setCurves] = useState<CurveMode>('custom');
  const auto = useAutoColor(edited, edit);
  const changed = useMemo(() => {
    const set = new Set<Palette>();
    if (!edit.enabled) return set;
    if (primariesChanged(edit.corrector)) set.add('primaries');
    if (curvesChanged(edit.corrector)) set.add('curves');
    return set;
  }, [edit.enabled, edit.corrector]);

  const keys = useRef({ run: auto.run });
  keys.current = { run: auto.run };
  useEffect(() => {
    const down = (event: globalThis.KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key) || key.repeat || !key.alt || key.ctrl || key.meta) return;
      const letter = key.key.length === 1 ? key.key.toLowerCase() : key.key;
      if (!key.shift && (key.code === 'KeyZ' || letter === 'z')) {
        // Alt+Z: between the Primaries and Log modes (p. 3068); from Bars, to Log.
        event.preventDefault();
        setPrimaries(mode => (mode === 'log' ? 'wheels' : 'log'));
      } else if (key.shift && (key.code === 'KeyC' || letter === 'c')) {
        event.preventDefault();
        keys.current.run();
      }
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, []);

  const filled: Filled = { edit, primaries, onPrimaries: setPrimaries, curves, onCurves: setCurves, onAuto: auto.run, status: auto.status, changed, scopes, windows };
  return <div className="color-palettes" data-merged={merged || undefined} role="region" aria-label={labels.palettes} data-test="palettes">
    {merged
      ? <PaletteGroup name={labels.palettes} palettes={both} chosen={last === 'left' ? left : center}
        onChoose={each => (isLeft(each) ? onLeft(each) : onCenter(each))} side="center" filled={filled} />
      : <>
        <PaletteGroup name={labels.leftPalettes} palettes={LEFT_PALETTES} chosen={left} onChoose={onLeft} side="left" filled={filled} />
        <PaletteGroup name={labels.centerPalettes} palettes={CENTER_PALETTES} chosen={center} onChoose={onCenter} side="center" filled={filled} />
      </>}
    <PaletteGroup name={labels.rightPanel} palettes={RIGHT_PANELS} chosen={right} onChoose={onRight} side="right" filled={filled} />
  </div>;
}
