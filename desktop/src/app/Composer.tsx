import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type RefObject } from 'react';
import { ArrowUp, Clock3, Square, SquareSlash } from 'lucide-react';
import { messageText } from '../state';
import { canSubmitInput } from '../uiActions';
import type { Workspace } from '../useWorkspace';
import { composerKey, keyOf } from './keys';
import { useWords } from './locale';
import { goneWords } from './tell';

/** How long the first Esc of two stays in mind. */
const ESC_TWICE_MS = 2000;
/** How high the field grows before it scrolls, in pixels. */
const MOST_HEIGHT = 220;

/**
 * The field where the user writes, with what belongs to it: the list of
 * commands, what still waits to be read, and the way to stop a turn.
 *
 * Args:
 *   draft, onDraft: What is written and not sent. The caller keeps it, one for each task.
 *   onSend: Send a line. It resolves to whether the line was taken.
 *   returned: Whether input that was dropped unread has just been put back into the field.
 *   declined: Whether the user has just said no to something that the agent asked for.
 *   commandsAsked: How often the list of commands was asked for from outside the field.
 */
export function Composer({ workspace, draft, onDraft, onSend, field, returned, declined, commandsAsked }: {
  workspace: Workspace;
  draft: string;
  onDraft: (text: string) => void;
  onSend: (text: string) => Promise<boolean>;
  field: RefObject<HTMLTextAreaElement | null>;
  returned: boolean;
  declined: boolean;
  commandsAsked: number;
}) {
  const words = useWords();
  const { status, hello, projection } = workspace;
  const running = status?.status === 'running';
  const stopped = status?.status === 'stopped';
  const unavailable = workspace.loading || !workspace.connected || stopped || !workspace.hasProject;
  const state = { unavailable, sending: workspace.sending, running };
  const canSend = canSubmitInput(draft, state);

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
  const submit = async () => {
    if (!canSend || submitting.current) return;
    submitting.current = true;
    try {
      await onSend(draft);
    } finally {
      submitting.current = false;
    }
  };
  const fill = (name: string) => {
    onDraft(`/${name} `);
    field.current?.focus();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    const action = composerKey(keyOf(event), { listOpen, running, cancelling: workspace.cancelling, armed });
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
        : workspace.loading || !workspace.connected ? words.label.placeholderStarting
          : running ? words.label.placeholderRunning : words.label.placeholderIdle;
  const hint = unavailable ? ''
    : workspace.cancelling ? words.label.stopping
      : armed ? words.hint.escAgain
        : returned ? words.hint.returned
          : declined && running ? words.hint.afterNo
            : running && draft.trim().startsWith('/') ? words.hint.commandsWhenIdle
              : running ? words.hint.keysRunning : words.hint.keysIdle;

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
      onSubmit={event => { event.preventDefault(); void submit(); }}
    >
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
        <p className="caption composer-hint" id="composer-hint" data-test="composer-hint" aria-live="polite">{hint}</p>
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
