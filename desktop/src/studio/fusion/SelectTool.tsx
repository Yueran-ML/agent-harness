import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { Blend, Circle, Droplet, Grid3x3, ImagePlus, Move, Pentagon, RectangleHorizontal, Square } from 'lucide-react';
import { keyOf, pickerKey } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { FusionKind } from '../engine';
import { ADDABLE, ADDABLE_EFFECTS, ADDABLE_MASKS } from './edits';
import { isMaskKind } from './graph';

/** A kind of node that the editor adds: every kind but the MediaOut, of which a composition has exactly one. */
export type AddableKind = Exclude<FusionKind, 'media_out'>;

/** The face of the button that adds a node of each kind, in the tool bar and in Select Tool. */
export const ADD_ICONS = {
  merge: Blend, transform: Move, background: Square, media_in: ImagePlus,
  rectangle: RectangleHorizontal, ellipse: Circle, polygon: Pentagon,
  blur: Droplet, mosaic: Grid3x3,
} as const;

/** Every kind that the editor adds, in the order of the tool bar: the nodes of a picture, the effects on one, the masks. */
export const TOOLS: readonly AddableKind[] = [...ADDABLE, ...ADDABLE_EFFECTS, ...ADDABLE_MASKS];

/**
 * The kinds that what was typed finds, those whose name begins with it before those that have it
 * further on, each in the order of the tool bar. A kind is found by the name that the page shows it
 * by, and by the name that the engine gives it (`media_in`, written with or without what parts its
 * words), so that `blur` finds a Blur in every language. Nothing typed finds every kind. Letters
 * are compared whatever their case.
 */
export function matchingTools(query: string, names: Readonly<Record<FusionKind, string>>): AddableKind[] {
  const asked = query.trim().toLowerCase();
  if (asked === '') return [...TOOLS];
  const joined = asked.replace(/[\s_]+/g, '');
  const rank = (kind: AddableKind): number => {
    const name = names[kind].toLowerCase();
    const own = kind.replace(/_/g, '');
    if (name.startsWith(asked) || (joined !== '' && own.startsWith(joined))) return 0;
    if (name.includes(asked) || (joined !== '' && own.includes(joined))) return 1;
    return 2;
  };
  return [...TOOLS.filter(kind => rank(kind) === 0), ...TOOLS.filter(kind => rank(kind) === 1)];
}

/**
 * Select Tool (Fusion's Shift+Space): a small dialog over the node editor that lists the kinds of
 * node that the editor adds, and adds the one chosen. What is typed narrows the list
 * (`matchingTools`); the arrows move through it, Enter takes the kind that is marked, a click
 * takes the one clicked, and Escape, a press elsewhere, or the keyboard gone elsewhere closes the
 * dialog and adds nothing.
 *
 * The keyboard stays in the field, and the list is worked from there, as the list of models is: the
 * field is a combobox that owns the list and says which of its options is marked
 * (`aria-activedescendant`).
 */
export function SelectTool({ onAdd, onClose }: {
  /** Add a node of the kind taken; the dialog is closed already. */
  onAdd: (kind: AddableKind) => void;
  onClose: () => void;
}) {
  const words = useWords();
  const labels = words.label.studio.fusionPage;
  const [query, setQuery] = useState('');
  const [marked, setMarked] = useState(0);
  const whole = useRef<HTMLDivElement>(null);
  const filter = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const listId = useId();
  const tools = useMemo(() => matchingTools(query, labels.kinds), [query, labels.kinds]);
  const active = tools.length === 0 ? -1 : Math.min(marked, tools.length - 1);

  // The keyboard goes into the field as the dialog opens.
  useEffect(() => { filter.current?.focus(); }, []);
  // The kind that is marked stays in sight where the list scrolls.
  useEffect(() => {
    if (active >= 0) list.current?.children[active]?.scrollIntoView?.({ block: 'nearest' });
  }, [active]);
  // A press anywhere else closes the dialog, and is what it is where it was made.
  useEffect(() => {
    const away = (event: Event) => {
      if (!(event.target instanceof Node) || !whole.current?.contains(event.target)) onClose();
    };
    document.addEventListener('pointerdown', away, true);
    return () => document.removeEventListener('pointerdown', away, true);
  }, [onClose]);

  const take = (kind: AddableKind) => {
    onClose();
    onAdd(kind);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const key = keyOf(event);
    const action = pickerKey(key);
    if (action === 'close') {
      // Escape closes the dialog and nothing else: a drag or a link chosen in the editor stays as it is.
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    // Enter that ends what an input method puts together is the input method's.
    if (event.key === 'Enter' && !key.composing) event.preventDefault();
    if (action === null) return;
    event.preventDefault();
    if (active < 0) return;
    if (action === 'take') take(tools[active]);
    else setMarked((active + (action === 'next' ? 1 : -1) + tools.length) % tools.length);
  };
  const groupOf = (kind: AddableKind): string | null => (isMaskKind(kind) ? labels.masks
    : (ADDABLE_EFFECTS as readonly FusionKind[]).includes(kind) ? labels.effects : null);

  return <div ref={whole} className="fusion-select-tool" role="dialog" aria-label={labels.selectTool} onKeyDown={onKeyDown}
    // The keyboard that leaves the dialog for somewhere else closes it.
    onBlur={(event) => {
      const next = event.relatedTarget;
      if (next instanceof Node && !event.currentTarget.contains(next)) onClose();
    }}
    data-test="select-tool">
    <input ref={filter} className="field fusion-select-tool-filter" value={query} placeholder={labels.searchTools} aria-label={labels.searchTools}
      role="combobox" aria-autocomplete="list" aria-expanded={tools.length > 0} aria-controls={tools.length > 0 ? listId : undefined}
      aria-activedescendant={active >= 0 ? `${listId}-${active}` : undefined} autoComplete="off" spellCheck={false}
      onChange={(event) => { setQuery(event.target.value); setMarked(0); }} data-test="select-tool-filter" />
    {tools.length === 0 && <p className="caption fusion-select-tool-none" role="status" data-test="select-tool-none">{words.hint.studio.fusionPage.noTool}</p>}
    {tools.length > 0 && <div ref={list} className="fusion-select-tool-list" role="listbox" id={listId} aria-label={labels.selectTool}>
      {tools.map((kind, index) => {
        const Icon = ADD_ICONS[kind];
        const group = groupOf(kind);
        return <button type="button" role="option" key={kind} id={`${listId}-${index}`} className="fusion-select-tool-option" data-kind={kind}
          aria-selected={index === active}
          // The keyboard stays in the field.
          tabIndex={-1} onMouseDown={event => event.preventDefault()} onMouseEnter={() => setMarked(index)} onClick={() => take(kind)}
          data-test="select-tool-option">
          <Icon size={14} aria-hidden="true" />
          <span className="fusion-select-tool-name">{labels.kinds[kind]}</span>
          {group !== null && <span className="fusion-select-tool-group">{group}</span>}
        </button>;
      })}
    </div>}
  </div>;
}
