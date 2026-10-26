import { useEffect, useLayoutEffect, useRef, useState, type DragEvent, type KeyboardEvent, type RefObject } from 'react';
import { ArrowUp, Clock3, FileImage, FileText, Paperclip, Square, SquareSlash, X } from 'lucide-react';
import { messageText } from '../state';
import { canSubmitInput, isCommand } from '../uiActions';
import type { ServiceModels } from '../useModels';
import type { Workspace } from '../useWorkspace';
import { EffortPicker } from './EffortPicker';
import { ModelPicker } from './ModelPicker';
import { isPicture, sizeOf, type Attachment, type Refused } from './attachments';
import { effortOf, type Effort } from './effort';
import { composerKey, keyOf } from './keys';
import { useWords } from './locale';
import { modifier } from './system';
import { goneWords, servicesToSwitchTo } from './tell';

/** How long the first Esc of two stays in mind. */
const ESC_TWICE_MS = 2000;
/** How high the field grows before it scrolls, in pixels. */
const MOST_HEIGHT = 220;

/** The kinds of picture that can be pasted to go with a message, as a screenshot is. */
const PASTED_PICTURES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];

/**
 * The field where the user writes, with what belongs to it: the list of
 * commands, what still waits to be read, and the way to stop a turn.
 *
 * Args:
 *   draft, onDraft: What is written and not sent. The caller keeps it, one for each task.
 *   onSend: Send a line, with `now` to have a turn that runs read it at once. It resolves to
 *     whether the line was taken.
 *   returned: Whether input that was dropped unread has just been put back into the field.
 *   declined: Whether the user has just said no to something that the agent asked for.
 *   commandsAsked: How often the list of commands was asked for from outside the field.
 *   models: The models of the services, as the page knows them.
 *   onChooseModel: Have the service process ask another model.
 *   onSwitchService: Have Columbina go on with a model of another service that is set up. Without
 *     it only the models of the service in use are offered.
 *   onChooseEffort: Have the model think as hard as another level says. Without it no level is offered.
 *   onModelSettings: Open the settings at the model.
 *   attachments: The files that go with the message. The caller keeps them, one set for each task.
 *   refused: The files that were chosen or dropped last and not taken, with why.
 *   onAttach: Let the user choose files to go with the message. Without it, as in a replay, no
 *     file goes with a message.
 *   onDropFiles: Take files that were dropped on the field.
 *   onPastePictures: Take pictures that were pasted into the field, where nothing else was pasted.
 *   onRemoveAttachment: Take a file out of the message.
 */
export function Composer({
  workspace, draft, onDraft, onSend, field, returned, declined, commandsAsked, models, onChooseModel, onSwitchService,
  onChooseEffort, onModelSettings, attachments, refused, onAttach, onDropFiles, onPastePictures, onRemoveAttachment,
}: {
  workspace: Workspace;
  draft: string;
  onDraft: (text: string) => void;
  onSend: (text: string, now?: boolean) => Promise<boolean>;
  field: RefObject<HTMLTextAreaElement | null>;
  returned: boolean;
  declined: boolean;
  commandsAsked: number;
  models: ServiceModels;
  onChooseModel: (model: string) => void;
  onSwitchService?: (service: string, model: string) => void;
  onChooseEffort?: (effort: Effort) => void;
  onModelSettings: () => void;
  attachments: readonly Attachment[];
  refused: readonly Refused[];
  onAttach?: () => void;
  onDropFiles?: (files: File[]) => void;
  onPastePictures?: (files: File[]) => void;
  onRemoveAttachment: (attachment: Attachment) => void;
}) {
  const words = useWords();
  const { status, hello, projection } = workspace;
  const running = status?.status === 'running';
  const stopped = status?.status === 'stopped';
  const unavailable = workspace.loading || !workspace.connected || stopped || !workspace.hasProject;
  // The model that the service process asks is known once it has said how the task stands.
  const provider = status?.model.provider ?? '';
  const model = status?.model.model ?? '';
  const canModel = (hello?.commands ?? []).some(command => command.name === 'model');
  // How hard the model thinks, where the program tells of the levels and takes the command.
  const thinking = effortOf(status?.model);
  const canEffort = (hello?.commands ?? []).some(command => command.name === 'effort');
  const state = { unavailable, sending: workspace.sending, running };
  // Files go with a message of their own where nothing is written. A command takes none.
  const canSend = canSubmitInput(draft, state)
    || (draft.trim() === '' && attachments.length > 0 && canSubmitInput('files', state));

  // Files that are dragged over the field are taken where they are let go of. The field is told
  // of every element inside it that the drag enters and leaves, and counts them.
  const [dropping, setDropping] = useState(false);
  const inside = useRef(0);
  const takesDrop = (event: DragEvent) => onDropFiles !== undefined && !unavailable
    && Array.from(event.dataTransfer.types).includes('Files');
  useEffect(() => { if (unavailable) { inside.current = 0; setDropping(false); } }, [unavailable]);

  // The list of commands is open while the line is a slash and the beginning of a name.
  const [closedFor, setClosedFor] = useState<string | null>(null);
  const [chosen, setChosen] = useState(0);
  const typed = draft.startsWith('/') && !/\s/.test(draft) ? draft.slice(1).toLowerCase() : null;
  const matching = typed === null ? [] : (hello?.commands ?? []).filter(command => command.name.startsWith(typed));
  const listOpen = matching.length > 0 && closedFor !== draft && !unavailable;
  const active = Math.min(chosen, matching.length - 1);
  useEffect(() => setChosen(0), [typed]);
  // Asked for from outside: the list opens again if it was closed, and on an empty field it
  // opens with every command. Whatever else is written stays as it is.
  useEffect(() => {
    if (commandsAsked === 0) return;
    setClosedFor(null);
    if (draft.trim() === '' && !running) onDraft('/');
    field.current?.focus();
  }, [commandsAsked]);

  // The first Esc of two. It is forgotten after a moment, and when the turn is over.
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), ESC_TWICE_MS);
    return () => clearTimeout(timer);
  }, [armed]);
  useEffect(() => { if (!running) setArmed(false); }, [running]);

  // The field is as high as what is written in it, up to a limit.
  useLayoutEffect(() => {
    const node = field.current;
    if (!node) return;
    node.style.height = 'auto';
    node.style.height = `${Math.min(MOST_HEIGHT, node.scrollHeight)}px`;
    // It scrolls only once it is as high as it gets. A field that was measured while it was
    // hidden would show a bar otherwise.
    node.style.overflowY = node.scrollHeight > MOST_HEIGHT ? 'auto' : 'hidden';
    // Measured again when the field comes into view, because a hidden one has no height.
  }, [draft, field, unavailable, workspace.hasProject, workspace.loading]);

  const submitting = useRef(false);
  const submit = async (now = false) => {
    if (!canSend || submitting.current) return;
    submitting.current = true;
    try {
      await (now ? onSend(draft, true) : onSend(draft));
    } finally {
      submitting.current = false;
    }
  };
  const fill = (name: string) => {
    onDraft(`/${name} `);
    field.current?.focus();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    const asking = workspace.approval !== null;
    const action = composerKey(keyOf(event), { listOpen, running, cancelling: workspace.cancelling, armed, asking });
    if (action === null) return;
    event.preventDefault();
    switch (action) {
      case 'next':
        setChosen((active + 1) % matching.length);
        break;
      case 'previous':
        setChosen((active - 1 + matching.length) % matching.length);
        break;
      case 'take':
        fill(matching[active].name);
        break;
      case 'closeList':
        setClosedFor(draft);
        break;
      case 'send':
        void submit();
        break;
      case 'sendNow':
        void submit(true);
        break;
      case 'arm':
        setArmed(true);
        break;
      case 'stop':
        setArmed(false);
        void workspace.cancel();
        break;
    }
  };

  const placeholder = !workspace.hasProject ? words.label.placeholderNoProject
    : workspace.closed !== null ? goneWords(words, workspace.hello !== null)
      : stopped ? words.label.placeholderStopped
        : !workspace.connected ? words.label.placeholderStarting
          : workspace.loading ? words.label.placeholderOpening
          : running ? words.label.placeholderRunning
            : workspace.chat ? words.label.placeholderChat : words.label.placeholderIdle;
  const hint = unavailable ? ''
    : workspace.cancelling ? words.label.stopping
      : armed ? words.hint.escAgain
        : returned ? words.hint.returned
          : declined && running ? words.hint.afterNo
            : attachments.length > 0 && isCommand(draft) ? words.hint.attachments.command
              : running && draft.trim().startsWith('/') ? words.hint.commandsWhenIdle
                : running ? words.hint.keysRunning(modifier()) : words.hint.keysIdle;

  // What a person said and nothing has read yet. What a plugin put in is not shown here.
  const waiting = [...projection.queues['next-turn'], ...projection.queues['next-step']]
    .filter(entry => entry.source === 'user' || entry.source === 'steer');

  return <div className="composer-area" data-test="composer-area">
    {waiting.length > 0 && <ul className="queue" data-test="queue" aria-label={words.label.waiting}>
      {waiting.map((entry, index) => <li key={index}>
        <Clock3 size={12} aria-hidden="true" />
        <span className="queue-text">{messageText(entry.message)}</span>
        <span className="caption">{words.label.waiting}</span>
      </li>)}
    </ul>}

    <form
      className="composer"
      data-running={running || undefined}
      data-off={unavailable || undefined}
      data-dropping={dropping || undefined}
      onSubmit={event => { event.preventDefault(); void submit(); }}
      onDragEnter={(event) => {
        if (!takesDrop(event)) return;
        event.preventDefault();
        inside.current += 1;
        setDropping(true);
      }}
      onDragOver={(event) => {
        if (!takesDrop(event)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = 'copy';
      }}
      onDragLeave={() => {
        inside.current = Math.max(0, inside.current - 1);
        if (inside.current === 0) setDropping(false);
      }}
      onDrop={(event) => {
        if (!takesDrop(event)) return;
        event.preventDefault();
        inside.current = 0;
        setDropping(false);
        const files = Array.from(event.dataTransfer.files);
        if (files.length > 0) onDropFiles!(files);
      }}
    >
      {dropping && <div className="composer-drop" aria-hidden="true" data-test="composer-drop">{words.hint.attachments.drop}</div>}
      {listOpen && <div className="commands" id="commands" role="listbox" aria-label={words.label.commands} data-test="commands">
        <p className="caption commands-note">{words.hint.commandsList}</p>
        {matching.map((command, index) => <button
          type="button"
          role="option"
          id={`command-${command.name}`}
          key={command.name}
          aria-selected={index === active}
          // The keyboard stays in the field, so the list is worked from there.
          tabIndex={-1}
          onMouseEnter={() => setChosen(index)}
          onClick={() => fill(command.name)}
        >
          <code>/{command.name}</code>
          <span>{command.summary}</span>
        </button>)}
      </div>}

      {attachments.length > 0 && <ul className="attachments" aria-label={words.label.attachments.list} data-test="attachments">
        {attachments.map(file => <li className="attachment" key={file.id} title={file.name} data-test="attachment">
          {isPicture(file.name) ? <FileImage size={13} aria-hidden="true" /> : <FileText size={13} aria-hidden="true" />}
          <span className="attachment-name">{file.name}</span>
          <span className="attachment-size">{sizeOf(file.size)}</span>
          <button
            type="button"
            className="attachment-remove"
            aria-label={words.label.attachments.remove(file.name)}
            title={words.label.attachments.remove(file.name)}
            onClick={() => { onRemoveAttachment(file); field.current?.focus(); }}
            data-test="attachment-remove"
          >
            <X size={12} />
          </button>
        </li>)}
      </ul>}
      {(refused.length > 0 || attachments.some(file => isPicture(file.name))) && <div className="attachments-notes" role="status" data-test="attachments-notes">
        {/* Of a picture the model has the file, and not what it shows. */}
        {attachments.some(file => isPicture(file.name)) && <p className="caption">{words.hint.attachments.picture}</p>}
        {refused.map(file => <p className="caption" key={`${file.why} ${file.name}`} data-tone="warn">
          {words.hint.attachments.refused[file.why](file.name)}
        </p>)}
      </div>}

      <label htmlFor="composer" className="sr-only">{placeholder}</label>
      <textarea
        id="composer"
        ref={field}
        data-test="composer"
        value={draft}
        rows={1}
        disabled={unavailable}
        placeholder={placeholder}
        spellCheck={false}
        aria-describedby="composer-hint"
        aria-autocomplete="list"
        aria-expanded={listOpen}
        aria-controls={listOpen ? 'commands' : undefined}
        aria-activedescendant={listOpen ? `command-${matching[active].name}` : undefined}
        onChange={event => { setClosedFor(null); onDraft(event.target.value); }}
        onKeyDown={onKeyDown}
        onPaste={event => {
          if (onPastePictures === undefined) return;
          // What comes with text, such as cells of a table that come as a picture too, is pasted as text.
          if (event.clipboardData.getData('text/plain') !== '') return;
          const pictures = Array.from(event.clipboardData.files).filter(file => PASTED_PICTURES.includes(file.type));
          if (pictures.length === 0) return;
          event.preventDefault();
          onPastePictures(pictures);
        }}
      />

      <div className="composer-row">
        <button
          type="button"
          className="icon-btn"
          disabled={unavailable || running}
          aria-label={words.label.commands}
          title={words.label.commands}
          onClick={() => { setClosedFor(null); onDraft('/'); field.current?.focus(); }}
        >
          <SquareSlash size={16} />
        </button>
        {onAttach && <button
          type="button"
          className="icon-btn"
          disabled={unavailable}
          aria-label={words.label.attachments.attach}
          title={words.label.attachments.attach}
          onClick={onAttach}
          data-test="attach"
        >
          <Paperclip size={16} />
        </button>}
        {provider !== '' && model !== '' && <ModelPicker
          provider={provider}
          model={model}
          services={workspace.services}
          models={models.of(provider)}
          choice={unavailable || !canModel ? 'off' : running || workspace.sending ? 'busy' : 'free'}
          replay={workspace.mode === 'mock'}
          placement="up"
          // The list of commands is put away while the panel stands where it would stand.
          onOpen={() => { setClosedFor(draft); models.freshen(provider); }}
          onAsk={() => void models.ask(provider)}
          onChoose={onChooseModel}
          onSettings={onModelSettings}
          afterChoice={() => field.current?.focus()}
          switching={onSwitchService && {
            services: servicesToSwitchTo(workspace.services, provider),
            models: models.of,
            onShow: models.freshen,
            onAsk: service => void models.ask(service),
            onSwitch: onSwitchService,
          }}
        />}
        {thinking && canEffort && onChooseEffort && workspace.mode !== 'mock' && <EffortPicker
          effort={thinking.effort}
          efforts={thinking.efforts}
          choice={unavailable ? 'off' : running || workspace.sending ? 'busy' : 'free'}
          placement="up"
          // The list of commands is put away while the panel stands where it would stand.
          onOpen={() => setClosedFor(draft)}
          onChoose={onChooseEffort}
          afterChoice={() => field.current?.focus()}
        />}
        {/* A narrow window cuts the line short, and the whole of it is shown on the way over it. */}
        <p className="caption composer-hint" id="composer-hint" data-test="composer-hint" aria-live="polite" title={hint}>{hint}</p>
        {running && <button
          type="button"
          className="btn stop"
          data-test="stop"
          disabled={workspace.cancelling}
          onClick={() => void workspace.cancel()}
        >
          <Square size={11} fill="currentColor" aria-hidden="true" />
          {workspace.cancelling ? words.label.stopping : words.label.stop}
        </button>}
        <button type="submit" className="send" data-test="send" disabled={!canSend} aria-label={words.label.send} title={words.label.send}>
          <ArrowUp size={17} strokeWidth={2.2} />
        </button>
      </div>
    </form>
  </div>;
}
