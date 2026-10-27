import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { Brain, Check, ChevronDown } from 'lucide-react';
import type { Effort } from './effort';
import { keyOf, pickerKey } from './keys';
import { useWords } from './locale';
import { useOverlay } from './overlay';
import type { ModelChoice } from './ModelPicker';
import { useRoom } from './room';

/**
 * How hard the model thinks, on a button beside the model, and the levels that the service offers
 * in a panel that the button opens. Each level says beneath its name what it does. The keyboard
 * goes into the list: the arrows move through it, Enter chooses, and Esc closes the panel and
 * gives the keyboard back to the button.
 *
 * Args:
 *   effort: The level in use.
 *   efforts: The levels that the service offers, the least first.
 *   choice: Whether a level can be chosen. While Columbina is at work the levels can be looked
 *     at, and none chosen.
 *   placement: Whether the panel opens above the button or below it, where it has the room.
 *   onOpen: The panel is opened.
 *   onChoose: Another level is chosen.
 *   afterChoice: Give the keyboard to where it goes once a level is chosen. The button keeps it otherwise.
 */
export function EffortPicker({ effort, efforts, choice, placement, onOpen, onChoose, afterChoice }: {
  effort: Effort;
  efforts: readonly Effort[];
  choice: ModelChoice;
  placement: 'up' | 'down';
  onOpen?: () => void;
  onChoose: (effort: Effort) => void;
  afterChoice?: () => void;
}) {
  const words = useWords();
  const [open, setOpen] = useState(false);
  // Its panel may reach over the viewer of the studio.
  useOverlay(open);
  // The level that the keyboard is on.
  const [marked, setMarked] = useState<Effort>(effort);
  const whole = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  // The panel stands within what holds it. Where it has too little room on the side that it is
  // asked to open on and more on the other, it opens there; where it has less than it needs, its
  // list scrolls.
  const { room, placement: side } = useRoom(panel, open, placement);
  const id = useId();
  const panelId = `${id}panel`;
  const listId = `${id}list`;
  // Where the service is not told how much to think, ultra is the work of a team alone.
  const thinks = efforts.includes('max');
  const noteOf = (level: Effort) => (level === 'ultra' && !thinks ? words.hint.effort.notes.ultraTeam : words.hint.effort.notes[level]);
  const active = Math.max(0, efforts.indexOf(marked));

  // A panel of which nothing can be used any more is closed.
  useEffect(() => { if (choice === 'off') setOpen(false); }, [choice]);
  // The keyboard goes into the list when the panel opens.
  useEffect(() => { if (open) list.current?.focus(); }, [open]);
  // The level that the keyboard is on stays in sight, where the list scrolls: once more when the
  // panel has been given the height that it has room for.
  useEffect(() => {
    if (open) list.current?.children[active]?.scrollIntoView({ block: 'nearest' });
  }, [active, open, room]);
  // A click anywhere else closes the panel, and leaves the keyboard where the click put it.
  useEffect(() => {
    if (!open) return undefined;
    const away = (event: Event) => {
      if (!whole.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', away);
    return () => document.removeEventListener('pointerdown', away);
  }, [open]);

  const show = () => {
    setMarked(effort);
    setOpen(true);
    onOpen?.();
  };
  const close = (giveBack: boolean) => {
    setOpen(false);
    if (giveBack) button.current?.focus();
  };
  const take = (chosen: Effort) => {
    if (choice !== 'free') return;
    close(false);
    if (chosen !== effort) onChoose(chosen);
    if (afterChoice) afterChoice();
    else button.current?.focus();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const action = pickerKey(keyOf(event));
    if (action === null) return;
    event.preventDefault();
    event.stopPropagation();
    if (action === 'close') close(true);
    else if (action === 'take') take(efforts[active]);
    else setMarked(efforts[(active + (action === 'next' ? 1 : -1) + efforts.length) % efforts.length]);
  };

  return <div
    className="model-picker effort-picker"
    ref={whole}
    // The keyboard that leaves the panel for somewhere else closes it.
    onBlur={(event) => {
      const next = event.relatedTarget;
      if (open && next instanceof Node && !event.currentTarget.contains(next)) setOpen(false);
    }}
  >
    <button
      type="button"
      ref={button}
      className="model-button"
      disabled={choice === 'off'}
      aria-haspopup="dialog"
      aria-expanded={open}
      aria-controls={open ? panelId : undefined}
      title={`${words.label.effort.title}: ${words.label.effort.names[effort]}`}
      onClick={() => (open ? close(false) : show())}
      data-test="effort-button"
    >
      <Brain size={13} aria-hidden="true" />
      <span className="model-button-name effort-button-name">{words.label.effort.button(words.label.effort.names[effort])}</span>
      <ChevronDown size={12} aria-hidden="true" />
    </button>

    {open && <div
      className="model-panel effort-panel"
      id={panelId}
      ref={panel}
      style={room === null ? undefined : { maxHeight: room }}
      role="dialog"
      aria-label={words.label.effort.title}
      data-placement={side}
      data-test="effort-panel"
    >
      <div className="model-head"><span className="model-service">{words.label.effort.title}</span></div>
      {choice === 'busy' && <div className="model-notes"><p className="caption">{words.hint.effort.whenIdle}</p></div>}
      <div
        className="model-list"
        role="listbox"
        id={listId}
        ref={list}
        tabIndex={0}
        aria-label={words.label.effort.title}
        aria-activedescendant={`${listId}-${active}`}
        onKeyDown={onKeyDown}
      >
        {efforts.map((level, index) => <div
          key={level}
          id={`${listId}-${index}`}
          role="option"
          className="model-option effort-option"
          data-level={level}
          aria-selected={index === active}
          aria-current={level === effort ? 'true' : undefined}
          aria-disabled={choice !== 'free' || undefined}
          onMouseDown={event => event.preventDefault()}
          onMouseEnter={() => setMarked(level)}
          onClick={() => take(level)}
          data-test="effort-option"
        >
          {level === effort ? <Check size={13} aria-hidden="true" /> : <span className="model-mark" aria-hidden="true" />}
          <span className="effort-words">
            <span className="effort-name">{words.label.effort.names[level]}</span>
            <span className="effort-note">{noteOf(level)}</span>
          </span>
        </div>)}
      </div>
    </div>}
  </div>;
}
