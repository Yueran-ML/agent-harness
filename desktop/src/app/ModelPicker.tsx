import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { Check, ChevronDown, CornerDownLeft, RotateCw, Settings2, Sparkles } from 'lucide-react';
import { called, looksLikeAKey, sentenceOf, serviceCalled, type Failure, type Listed, type ListedService } from '../key/form';
import type { Messages } from '../messages/en';
import type { Models } from '../useModels';
import { keyOf, pickerKey } from './keys';
import { useWords } from './locale';
import { useOverlay } from './overlay';
import { filtered, typedModel } from './models';
import { useRoom } from './room';

/** Whether a model can be chosen: now, once Columbina is at rest, or not here at all. */
export type ModelChoice = 'free' | 'busy' | 'off';

/** What the panel needs to offer the models of the other services that are set up. */
export interface Switching {
  /** The services that can be switched to, the one in use among them, in the order of the list. */
  services: ListedService[];
  /** What is known of the models of a service. */
  models: (service: string) => Models | null;
  /** A service is shown: its models are asked for where what is known of them is old. */
  onShow: (service: string) => void;
  /** The models of a service are asked for again. */
  onAsk: (service: string) => void;
  /** A model of another service is chosen: Columbina goes on with that service. */
  onSwitch: (service: string, model: string) => void;
}

/**
 * Say why a service listed no models. A service on this computer that does not answer has not
 * been started, and the user is told how to start it.
 */
export function sayListFailed(words: Messages, failure: Failure, said: string, service: ListedService | null): string {
  if (service !== null && service.key_env === null && failure === 'network') {
    const known = service.name === 'ollama' || service.name === 'lmstudio' ? service.name : 'other';
    return words.hint.models.notRunning[known];
  }
  const sentence = words.hint.models.failed[failure];
  return failure === 'unknown' && said ? `${sentence} ${sentenceOf(said)}` : sentence;
}

/** The words of the option that uses what was written, with the model written as names of models are. */
function Written({ words, model }: { words: string; model: string }) {
  const at = words.indexOf(model);
  if (at < 0) return <>{words}</>;
  return <>{words.slice(0, at)}<span className="mono">{model}</span>{words.slice(at + model.length)}</>;
}

/**
 * The model that the service process asks, on a button, and the models of its service in a
 * panel that the button opens: the field has one at its foot, and the settings have one.
 *
 * The panel is a filter over a list. What the filter keeps is offered closest first, and a name
 * that the service does not list is offered as it is written, so that a model can be used where
 * the service lists none or cannot be asked. The keyboard stays in the filter: the arrows move
 * through the list, Enter chooses, and Esc closes the panel and gives the keyboard back to the
 * button.
 *
 * Where more than one service is set up, the head of the panel names each of them, and shows the
 * models of the one that is pressed: the one in use when the panel opens. A model of another
 * service is chosen as one of the service in use is, and Columbina goes on with that service.
 *
 * Args:
 *   provider: The name of the service that the service process talks to.
 *   model: The model that it asks.
 *   services: The services that are set up, where the page has the list. They say what the
 *     service is called, and whether it runs on this computer.
 *   models: What is known of the models of the service.
 *   choice: Whether a model can be chosen. While Columbina is at work the list can be looked
 *     at, and nothing in it chosen.
 *   replay: Whether the page shows a replay, which no service answers.
 *   placement: Whether the panel opens above the button or below it, where it has the room.
 *   onOpen: The panel is opened. The models are asked for where what is known of them is old.
 *   onAsk: The models are asked for again.
 *   onChoose: Another model is chosen.
 *   onSettings: Lead to the settings of the model. Without it the panel has no way there.
 *   afterChoice: Give the keyboard to where it goes once a model is chosen. The button keeps it otherwise.
 *   switching: How the other services that are set up are offered. Without it, or in a replay, only
 *     the models of the service in use are.
 */
export function ModelPicker({
  provider, model, services, models, choice, replay, placement, onOpen, onAsk, onChoose, onSettings, afterChoice, switching,
}: {
  provider: string;
  model: string;
  services: Listed | null;
  models: Models | null;
  choice: ModelChoice;
  replay: boolean;
  placement: 'up' | 'down';
  onOpen: () => void;
  onAsk: () => void;
  onChoose: (model: string) => void;
  onSettings?: () => void;
  afterChoice?: () => void;
  switching?: Switching;
}) {
  const words = useWords();
  const [open, setOpen] = useState(false);
  // Its panel may reach over the viewer of the studio.
  useOverlay(open);
  // The service whose models are shown. Where it is none of those offered, they are those of the service in use.
  const [looked, setLooked] = useState(provider);
  const [query, setQuery] = useState('');
  // The model that the keyboard is on, by its name. Where it is none of those offered, the
  // keyboard is on the model in use, or on the first one that the filter keeps.
  const [marked, setMarked] = useState<string | null>(null);
  const whole = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const filter = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const id = useId();
  const panelId = `${id}panel`;
  const listId = `${id}list`;
  // The panel stands within what holds it. Where it has too little room on the side that it is
  // asked to open on and more on the other, it opens there; where it has less than it needs, its
  // list scrolls.
  const { room, placement: side } = useRoom(panel, open, placement);

  // The services are offered where the one in use is among them, with another one to switch to.
  const offered = !replay && switching && switching.services.length > 1
    && switching.services.some(each => each.name === provider) ? switching : null;
  const showing = offered?.services.some(each => each.name === looked) ? looked : provider;
  // Where the models of another service are shown, what is done with them is for switching to it.
  const other = offered !== null && showing !== provider ? offered : null;
  const elsewhere = other !== null;
  const service = offered?.services.find(candidate => candidate.name === showing)
    ?? services?.services.find(candidate => candidate.name === showing) ?? null;
  const name = service ? called(service) : showing;
  const known = other ? other.models(showing) : models;
  const asking = !replay && (known === null || known.kind === 'asking');
  const failed = !replay && known?.kind === 'failed' ? known : null;
  const listed = known === null || known.kind === 'failed' ? [] : known.models;
  const written = query.trim();
  const shown = replay ? [] : filtered(listed, query);
  const typed = replay ? null : typedModel(listed, query);
  const options = [
    ...shown.map(each => ({ model: each, typed: false })),
    ...(typed === null ? [] : [{ model: typed, typed: true }]),
  ];
  const markedAt = options.findIndex(option => option.model === marked);
  const inUseAt = written === '' && !elsewhere ? options.findIndex(option => option.model === model) : -1;
  const active = options.length === 0 ? -1 : markedAt >= 0 ? markedAt : Math.max(0, inUseAt);

  // A panel of which nothing can be used any more is closed.
  useEffect(() => { if (choice === 'off' && !replay) setOpen(false); }, [choice, replay]);
  // The models are asked for when the panel opens, and the keyboard goes to the filter.
  useEffect(() => {
    if (!open) return;
    if (!replay) onOpen();
    (filter.current ?? panel.current)?.focus();
  }, [open]);
  // A click anywhere else closes the panel, and leaves the keyboard where the click put it.
  useEffect(() => {
    if (!open) return undefined;
    const away = (event: Event) => {
      if (!whole.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', away);
    return () => document.removeEventListener('pointerdown', away);
  }, [open]);
  // The model that the keyboard is on stays in sight: once more when the panel has been given
  // the height that it has room for.
  useEffect(() => {
    if (active >= 0) list.current?.children[active]?.scrollIntoView({ block: 'nearest' });
  }, [active, open, room]);

  const show = () => {
    setQuery('');
    setMarked(null);
    setLooked(provider);
    setOpen(true);
  };
  // Another service is shown, with the keyboard in the filter.
  const look = (at: string) => {
    if (!offered) return;
    setQuery('');
    setMarked(null);
    setLooked(at);
    offered.onShow(at);
    filter.current?.focus();
  };
  const askAgain = () => {
    if (other) other.onAsk(showing);
    else onAsk();
  };
  const close = (giveBack: boolean) => {
    setOpen(false);
    if (giveBack) button.current?.focus();
  };
  const take = (chosen: string) => {
    if (choice !== 'free') return;
    close(false);
    if (other) other.onSwitch(showing, chosen);
    else if (chosen !== model) onChoose(chosen);
    if (afterChoice) afterChoice();
    else button.current?.focus();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const key = keyOf(event);
    const action = pickerKey(key);
    if (action === 'close') {
      // Esc closes the panel and nothing else: in the settings, the settings stay open.
      event.preventDefault();
      event.stopPropagation();
      close(true);
      return;
    }
    // The other keys are those of the filter. A button in the panel takes Enter itself.
    if (event.target !== filter.current) return;
    // Enter in the filter is never that of a form around it, such as the field, which would send
    // what it holds. One that ends what an input method puts together is the input method's.
    if (event.key === 'Enter' && !key.composing) event.preventDefault();
    if (action === null) return;
    event.preventDefault();
    if (options.length === 0) return;
    if (action === 'take') take(options[active].model);
    else setMarked(options[(active + (action === 'next' ? 1 : -1) + options.length) % options.length].model);
  };

  return <div
    className="model-picker"
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
      disabled={choice === 'off' && !replay}
      aria-haspopup="dialog"
      aria-expanded={open}
      aria-controls={open ? panelId : undefined}
      aria-label={words.label.models.button(model)}
      title={`${serviceCalled(provider, services)} · ${model}`}
      onClick={() => (open ? close(false) : show())}
      data-test="model-button"
    >
      <Sparkles size={13} aria-hidden="true" />
      <span className="model-button-name">{model}</span>
      <ChevronDown size={12} aria-hidden="true" />
    </button>

    {open && <div
      className="model-panel"
      id={panelId}
      ref={panel}
      style={room === null ? undefined : { maxHeight: room }}
      role="dialog"
      aria-label={words.label.model}
      tabIndex={-1}
      data-placement={side}
      onKeyDown={onKeyDown}
      data-test="model-panel"
    >
      <div className="model-head">
        {/* A service of the user's own is known by its name, and where it is shows when it is pointed at. */}
        {offered ? <div className="model-services" role="group" aria-label={words.label.models.services}>
          {offered.services.map(each => <button
            type="button"
            key={each.name}
            className="chip model-service-choice"
            aria-pressed={each.name === showing}
            aria-label={each.name === provider ? words.label.models.inUse(called(each)) : undefined}
            title={each.own ? each.base_url : undefined}
            data-in-use={each.name === provider || undefined}
            onClick={() => look(each.name)}
            data-test="model-service-choice"
          >
            {called(each)}
          </button>)}
        </div>
          : <span className="model-service" title={service?.own ? service.base_url : undefined}>{name}</span>}
        {!replay && <button
          type="button"
          className="icon-btn"
          aria-label={words.label.models.again}
          title={words.label.models.again}
          // It stays where it is while the service is asked, and keeps the keyboard if it has it.
          onClick={() => { if (known?.kind !== 'asking') askAgain(); }}
          data-test="models-again"
        >
          <RotateCw size={13} className={asking ? 'spin' : undefined} />
        </button>}
      </div>

      {!replay && <input
        ref={filter}
        className="field model-filter"
        value={query}
        placeholder={words.label.models.filter}
        aria-label={words.label.models.filter}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={options.length > 0}
        aria-controls={options.length > 0 ? listId : undefined}
        aria-activedescendant={active >= 0 ? `${listId}-${active}` : undefined}
        autoComplete="off"
        spellCheck={false}
        onChange={(event) => { setQuery(event.target.value); setMarked(null); }}
        data-test="models-filter"
      />}

      <div className="model-notes" aria-live="polite" data-test="models-notes">
        {replay && <p className="caption">{words.hint.models.unavailable}</p>}
        {!replay && choice === 'busy' && <p className="caption">{words.hint.models.whenIdle}</p>}
        {elsewhere && choice === 'free' && <p className="caption">{words.hint.models.switch(name)}</p>}
        {asking && listed.length === 0 && <p className="caption model-asking">
          <span className="moon" aria-hidden="true" />{words.hint.models.asking(name)}
        </p>}
        {failed && <p className="caption">{sayListFailed(words, failed.failure, failed.said, service)}</p>}
        {known?.kind === 'listed' && listed.length === 0 && <p className="caption">{words.hint.models.none(name)}</p>}
        {/* A key that was pasted into the filter is not offered as the name of a model, and the user is told why. */}
        {!replay && written !== '' && looksLikeAKey(written) ? <p className="caption">{words.hint.key.likeAKey}</p>
          : listed.length > 0 && written !== '' && shown.length === 0 && <p className="caption">{words.hint.models.noMatch}</p>}
      </div>
      {failed && <button
        type="button"
        className="btn btn-quiet model-again"
        // The button goes away while the service is asked, and the keyboard goes back to the filter.
        onClick={() => { askAgain(); filter.current?.focus(); }}
        data-test="models-try-again"
      >
        {words.label.models.tryAgain}
      </button>}

      {options.length > 0 && <div className="model-list" role="listbox" id={listId} ref={list} aria-label={words.label.model}>
        {options.map((option, index) => <button
          type="button"
          role="option"
          key={`${option.typed ? 'typed' : 'listed'} ${option.model}`}
          id={`${listId}-${index}`}
          className="model-option"
          data-typed={option.typed || undefined}
          aria-selected={index === active}
          aria-current={!elsewhere && !option.typed && option.model === model ? 'true' : undefined}
          aria-disabled={choice !== 'free' || undefined}
          // The keyboard stays in the filter, and the list is worked from there.
          tabIndex={-1}
          title={option.model}
          onMouseDown={event => event.preventDefault()}
          onMouseEnter={() => setMarked(option.model)}
          onClick={() => take(option.model)}
        >
          {option.typed ? <CornerDownLeft size={13} aria-hidden="true" />
            : !elsewhere && option.model === model ? <Check size={13} aria-hidden="true" />
              : <span className="model-mark" aria-hidden="true" />}
          <span className="model-option-name">
            {option.typed ? <Written words={words.label.models.use(option.model)} model={option.model} /> : option.model}
          </span>
        </button>)}
      </div>}

      {onSettings && <div className="model-foot">
        <button
          type="button"
          className="model-settings"
          // The keyboard goes back to the button first, so that it comes back there when the settings close.
          onClick={() => { close(true); onSettings(); }}
          data-test="model-settings"
        >
          <Settings2 size={13} aria-hidden="true" />{words.label.models.settings}
        </button>
      </div>}
    </div>}
  </div>;
}
