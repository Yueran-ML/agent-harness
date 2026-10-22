import { useEffect, useMemo, useRef, useState } from 'react';
import { ChevronRight, Download, FileDiff, FileText, ListTree, MessageSquare, PanelLeft, Settings2, SlidersHorizontal, X } from 'lucide-react';
import { MusicTitlebar, useBackgroundMusic } from '../BackgroundMusic';
import { StartupVideo } from '../StartupVideo';
import type { ActionKey } from '../outcomes';
import { messageText, type UserItem } from '../state';
import { initialSize, storeSize, type SizeId } from '../size';
import { initialTheme, storeTheme, type ThemeId } from '../themes';
import type { Turn } from '../turns';
import {
  canSubmitInput, isCommand, permissionNeedsConfirmation, permissionPresetFromCommand, type PermissionPreset,
} from '../uiActions';
import { sentenceOf, storedOf } from '../key/form';
import { useGitHub } from '../useGitHub';
import { useModels } from '../useModels';
import { useUpdates } from '../useUpdates';
import { useWorkspace, type Workspace } from '../useWorkspace';
import { ApprovalCard } from './ApprovalCard';
import { Composer } from './Composer';
import { Conversation } from './Conversation';
import { changesOf, Inspector, type InspectorTab } from './Inspector';
import { Settings, type SettingsPart } from './Settings';
import { Sheet } from './Sheet';
import { Sidebar } from './Sidebar';
import { Recent, StartHead, StartSkills } from './Start';
import { StatusLine } from './StatusLine';
import { TraceView } from './TraceView';
import {
  added, Attachments, attachedOf, keptOf, withAttachments, type Attachment, type Refused,
} from './attachments';
import { Drafts, samePlace, Unread, type Place } from './drafts';
import { effortStoredOf, keptLevel, type Effort } from './effort';
import { shownLanguage, storedLanguage, storeLanguage, useWords, Words, wordsFor, type LanguageChoice } from './locale';
import { storedOpening, storeOpening } from './opening';
import { onMac, shortcut } from './system';
import {
  anotherServiceChosen, folderName, goneWords, joined, retryText, skillLine, STILL_STARTING_MS, taskTitle, waitingWords,
} from './tell';
import { storeChecking, storedChecking, storedSkipped, storeSkipped, toldOf } from './updates';

const tellingKey = 'columbina-tell-me';

/**
 * The line that stands while the page waits for the service. A start that takes long says
 * so, and can be given up then: a service that starts and never answers would otherwise
 * leave no way out but to close the window.
 */
function Starting({ workspace }: { workspace: Workspace }) {
  const words = useWords();
  const answered = workspace.hello !== null;
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    setSlow(false);
    if (answered) return undefined;
    const timer = setTimeout(() => setSlow(true), STILL_STARTING_MS);
    return () => clearTimeout(timer);
  }, [answered, workspace.folder]);
  return <div className="starting" data-test="starting">
    <p className="working" role="status">
      <span className="moon" aria-hidden="true" />
      {waitingWords(words, { answered, folder: workspace.folder, reading: workspace.reading })}
      {slow && <span className="working-more" data-test="still-starting">{words.hint.stillStarting}</span>}
    </p>
    {slow && <button type="button" className="btn btn-quiet" onClick={() => void workspace.cancelStart()} data-test="cancel-start">
      {words.label.cancel}
    </button>}
  </div>;
}

/** Whether the user is told of what waits for them while the window is in the background. */
function storedTelling(): boolean {
  try {
    return localStorage.getItem(tellingKey) !== 'false';
  } catch {
    return true;
  }
}

/** A change of what the task may do that waits for a yes. */
interface PendingPermission {
  preset: PermissionPreset;
  line: string;
  sessionId: string | null;
  /** Whether the change was made in the settings, where it shows without a line in the conversation. */
  quiet: boolean;
}

/** How a task is shown: as its conversation, or as its trace. */
type TaskView = 'conversation' | 'trace';

export default function App() {
  const workspace = useWorkspace();
  const github = useGitHub();
  const models = useModels();
  const music = useBackgroundMusic();
  const { projection, status } = workspace;

  const [theme, setTheme] = useState<ThemeId>(initialTheme);
  const [size, setSize] = useState<SizeId>(initialSize);
  const [language, setLanguage] = useState<LanguageChoice>(storedLanguage);
  const words = useMemo(() => wordsFor(language), [language]);
  // The opening plays at the first start. After that it plays where the user chose to see it
  // at every start, which is what the switch in the settings says.
  const [opening] = useState(storedOpening);
  const [playOpening, setPlayOpening] = useState(opening.always);
  const [showOpening, setShowOpening] = useState(opening.show);
  const [tellMe, setTellMe] = useState(storedTelling);
  // Newer versions of the application: whether it looks for them, and the one the user skipped.
  const [checkingUpdates, setCheckingUpdates] = useState(storedChecking);
  const [skipped, setSkipped] = useState(storedSkipped);
  const updates = useUpdates(checkingUpdates);
  const told = toldOf(updates.found, skipped);

  const [sidebarHidden, setSidebarHidden] = useState(false);
  // On macOS the buttons of the window are hidden in full screen, and the bar gives their room back.
  const [fullScreen, setFullScreen] = useState(false);
  useEffect(() => (onMac() ? window.columbina!.onFullScreen(setFullScreen) : undefined), []);
  const [inspector, setInspector] = useState<InspectorTab | null>(null);
  const [settings, setSettings] = useState<SettingsPart | null>(null);
  // What each task was last looked at as, by its id: its conversation, or its trace. A task
  // that was not looked at yet shows its conversation.
  const [views, setViews] = useState<Readonly<Record<string, TaskView>>>({});
  // A model or a level that was chosen and is in use, and could not be kept: what the settings say of it.
  const [notKept, setNotKept] = useState<string | null>(null);
  const [link, setLink] = useState<string | null>(null);
  const [pendingPermission, setPendingPermission] = useState<PendingPermission | null>(null);
  // Something that would stop the turn that is under way, and waits for a yes.
  const [pendingLeave, setPendingLeave] = useState<(() => void) | null>(null);
  // How often the list of commands was asked for with the keyboard.
  const [commandsAsked, setCommandsAsked] = useState(0);

  const field = useRef<HTMLTextAreaElement>(null);
  const firstButton = useRef<HTMLButtonElement>(null);
  const restartButton = useRef<HTMLButtonElement>(null);

  const running = status?.status === 'running';
  const stopped = status?.status === 'stopped';
  const unavailable = workspace.loading || !workspace.connected || stopped || !workspace.hasProject;

  // --- What the window looks like, and the language it speaks.
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    storeTheme(theme);
    void window.columbina?.setTheme(theme).catch(workspace.fail);
  }, [theme, workspace.fail]);
  useEffect(() => {
    document.documentElement.dataset.size = size;
    storeSize(size);
  }, [size]);
  useEffect(() => {
    // The language that is shown, which is English where the system has one that there are no words for.
    document.documentElement.lang = shownLanguage(language);
    // The menu of the tray speaks it too. What goes wrong there is nothing to the page.
    void Promise.resolve().then(() => window.columbina?.setLanguage(shownLanguage(language))).catch(() => {});
    storeLanguage(language);
  }, [language]);
  useEffect(() => {
    try {
      localStorage.setItem(tellingKey, String(tellMe));
    } catch {
      // Without storage the choice holds for this window.
    }
  }, [tellMe]);
  useEffect(() => storeChecking(checkingUpdates), [checkingUpdates]);
  const skip = (version: string) => {
    setSkipped(version);
    storeSkipped(version);
  };
  // Kept from the first moment on: a start that came this far has had its opening.
  useEffect(() => storeOpening(playOpening), [playOpening]);
  // A question of the agent is not kept waiting behind a film.
  useEffect(() => { if (workspace.approval) setShowOpening(false); }, [workspace.approval]);

  // --- What is written and not sent. Every task of every project keeps its own.
  const [draft, setDraft] = useState('');
  const [returned, setReturned] = useState(false);
  const drafts = useRef(new Drafts());
  const draftNow = useRef(draft);
  draftNow.current = draft;
  const returnedNow = useRef(returned);
  returnedNow.current = returned;
  // The files that go with the message, and those that were not taken. Every task keeps its own
  // files, as it keeps its draft.
  const attachmentsKept = useRef(new Attachments());
  const [attached, setAttached] = useState<Attachment[]>([]);
  const [refused, setRefused] = useState<Refused[]>([]);
  const attachedNow = useRef(attached);
  attachedNow.current = attached;
  const project = workspace.hello?.cwd ?? null;
  const place = useMemo<Place | null>(
    () => (workspace.sessionId === null ? null : { project, task: workspace.sessionId }),
    [project, workspace.sessionId],
  );
  const placeNow = useRef<Place | null>(null);
  // What waits unread in the task that is shown. The service drops it without a word when a
  // task is left while its turn is under way, so it is kept with the task as a part of its draft.
  const unread = useRef(new Unread());
  const waiting = [...projection.queues['next-turn'], ...projection.queues['next-step']]
    .filter(entry => entry.source === 'user' || entry.source === 'steer')
    .map(entry => messageText(entry.message));
  unread.current.see(place, running, waiting);
  useEffect(() => {
    const left = unread.current.take(place, running, waiting);
    if (placeNow.current !== null) {
      drafts.current.keep(placeNow.current, joined([...left, draftNow.current]), returnedNow.current || left.length > 0);
      attachmentsKept.current.keep(placeNow.current, attachedNow.current);
    }
    placeNow.current = place;
    setDraft(place === null ? '' : drafts.current.of(place));
    setAttached(place === null ? [] : attachmentsKept.current.of(place));
    setRefused([]);
    // What came back unread is still what the field holds, and the field goes on saying so.
    setReturned(place !== null && drafts.current.returned(place));
    setPendingPermission(null);
  }, [place]);
  // Whether the user has just said no to what the agent asked for. The field says then what can be done.
  const [declined, setDeclined] = useState(false);
  const write = (text: string) => { setReturned(false); setDeclined(false); setDraft(text); };
  useEffect(() => { if (!running) setDeclined(false); }, [running]);
  // A question that is answered goes away, and with it the button that had the keyboard.
  useEffect(() => {
    if (workspace.approval === null && document.activeElement === document.body) field.current?.focus();
  }, [workspace.approval]);

  // --- What was said and dropped unread comes back into the field. What a log
  // tells of earlier times is left alone: only what is dropped now comes back.
  const returnedUpTo = useRef(-1);
  useEffect(() => {
    returnedUpTo.current = workspace.openedAt - 1;
  }, [workspace.sessionId, workspace.openedAt]);
  useEffect(() => {
    const fresh = projection.dropped.filter(input => input.seq > returnedUpTo.current
      && (input.source === 'user' || input.source === 'steer'));
    if (fresh.length === 0) return;
    returnedUpTo.current = Math.max(...fresh.map(input => input.seq));
    setDraft(current => joined([...fresh.map(input => input.text), current]));
    setReturned(true);
    field.current?.focus();
  }, [projection.dropped]);

  // --- Sending.
  const state = { unavailable, sending: workspace.sending, running };
  const send = async (line: string, quiet = false): Promise<boolean> => {
    if (!canSubmitInput(line, state)) return false;
    const preset = permissionPresetFromCommand(line);
    if (preset && permissionNeedsConfirmation(preset, status)) {
      setPendingPermission({ preset, line, sessionId: workspace.sessionId, quiet });
      return false;
    }
    return workspace.send(line, quiet);
  };
  /** What was sent from a task is no draft of it any more, wherever the user is by the time the answer comes. */
  const forget = (from: Place | null, line: string) => {
    if (from === null) return;
    if (samePlace(from, placeNow.current)) {
      setDraft(current => (current === line ? '' : current));
      // Whatever came back unread has been sent again, or was written over before this.
      setReturned(false);
    } else drafts.current.sent(from, line);
  };
  const sendDraft = async (line: string): Promise<boolean> => {
    const from = placeNow.current;
    const files = attachedNow.current;
    // A command takes no files: they wait for the next message.
    if (files.length === 0 || isCommand(line)) {
      const taken = await send(line);
      if (taken) forget(from, line);
      return taken;
    }
    if (!window.columbina || !canSubmitInput('files', state)) return false;
    // The files are copied into the project first, and the message names where they are.
    let kept: string[] | null = null;
    let said = '';
    try {
      kept = keptOf(await window.columbina.keepAttachments(files.map(file => file.id)), files.length);
    } catch (problem) {
      said = problem instanceof Error ? problem.message : String(problem);
    }
    if (kept === null) {
      workspace.fail(`${words.hint.attachments.notKept} ${sentenceOf(said)}`.trim());
      return false;
    }
    const taken = await send(withAttachments(line, words.label.attachments.heading, kept));
    if (!taken) return false;
    forget(from, line);
    // The files went with the message. The shell lets go of them, and the task has them no more.
    void window.columbina.forgetAttachments(files.map(file => file.id)).catch(() => {});
    if (samePlace(from, placeNow.current)) setAttached([]);
    else if (from !== null) attachmentsKept.current.keep(from, []);
    setRefused([]);
    return taken;
  };
  /** Take the files that the shell holds now, as many as go with the message, and say of the others why not. */
  const takeAttached = (answer: unknown) => {
    const { attached: coming, refused: notTaken } = attachedOf(answer);
    const { files, over } = added(attachedNow.current, coming);
    if (over.length > 0) void window.columbina?.forgetAttachments(over.map(file => file.id)).catch(() => {});
    setAttached(files);
    setRefused([...notTaken, ...over.map(file => ({ name: file.name, why: 'tooMany' as const }))]);
  };
  // Files go with a message where the application runs the service itself, and not in a replay.
  const canAttach = window.columbina !== undefined && workspace.mode === 'service';
  const chooseAttachments = async () => {
    try {
      const answer = await window.columbina!.chooseAttachments({ language: shownLanguage(language) });
      if (answer !== null) takeAttached(answer);
    } catch (problem) {
      workspace.fail(problem);
    }
    field.current?.focus();
  };
  const dropAttachments = async (files: File[]) => {
    try {
      takeAttached(await window.columbina!.attachDropped(files));
    } catch (problem) {
      workspace.fail(problem);
    }
    field.current?.focus();
  };
  const removeAttachment = (file: Attachment) => {
    setAttached(current => current.filter(each => each.id !== file.id));
    setRefused([]);
    void window.columbina?.forgetAttachments([file.id]).catch(() => {});
  };
  /**
   * Have the service process ask another model from its next request on, and keep the model as
   * that of its service, for the next start. Where another service was chosen since and waits for
   * the next start, nothing is kept: what is stored for a service chooses that service.
   */
  const chooseModel = async (model: string) => {
    const provider = status?.model.provider;
    if (!provider || !(await send(`/model ${model}`, true))) return;
    setNotKept(null);
    if (!window.columbina || anotherServiceChosen(workspace.services, provider)) return;
    let said: string;
    try {
      const told = storedOf(await window.columbina.chooseModel({ service: provider, model }));
      if (told.kind === 'stored') {
        void workspace.refreshServices();
        return;
      }
      said = told.said;
    } catch (problem) {
      said = problem instanceof Error ? problem.message : String(problem);
    }
    // The model is in use all the same. The user is told that it is not kept, also in the settings.
    const notice = `${words.hint.models.notKept} ${sentenceOf(said)}`.trim();
    workspace.fail(notice);
    setNotKept(notice);
  };
  /**
   * Have Columbina go on with a model of another service. The model is kept as that of the service,
   * which chooses the service, and the service process is started again at the task that is open,
   * as it is after a change in the window of the key. It is offered at rest only.
   */
  const switchService = async (service: string, model: string) => {
    if (!window.columbina) return;
    setNotKept(null);
    let said: string;
    try {
      const told = storedOf(await window.columbina.chooseModel({ service, model }));
      if (told.kind === 'stored') {
        void workspace.refreshServices();
        void workspace.reconnect();
        return;
      }
      said = told.said;
    } catch (problem) {
      said = problem instanceof Error ? problem.message : String(problem);
    }
    // Nothing was changed. The user is told so, also in the settings.
    const notice = `${words.hint.models.notSwitched} ${sentenceOf(said)}`.trim();
    workspace.fail(notice);
    setNotKept(notice);
  };
  /**
   * Have the service process ask for another level of thinking from its next request on, and keep
   * the level for its service, for the next start. Ultra holds for this run, and is not kept.
   */
  const chooseEffort = async (effort: Effort) => {
    const provider = status?.model.provider;
    if (!provider || !(await send(`/effort ${effort}`, true))) return;
    setNotKept(null);
    if (!window.columbina || !keptLevel(effort)) return;
    let said: string;
    try {
      const told = effortStoredOf(await window.columbina.chooseEffort({ service: provider, effort }));
      if (told.kind === 'stored') return;
      said = told.said;
    } catch (problem) {
      said = problem instanceof Error ? problem.message : String(problem);
    }
    // The level is in use all the same. The user is told that it is not kept, also in the settings.
    const notice = `${words.hint.effort.notKept} ${sentenceOf(said)}`.trim();
    workspace.fail(notice);
    setNotKept(notice);
  };
  const confirmPermission = async () => {
    const change = pendingPermission;
    setPendingPermission(null);
    if (!change || change.sessionId !== workspace.sessionId || !canSubmitInput(change.line, state)) return;
    const from = placeNow.current;
    if (await workspace.send(change.line, change.quiet)) forget(from, change.line);
  };

  // --- Leaving a task. A turn that is under way is not stopped without a yes.
  const leave = (go: () => void) => {
    if (running) setPendingLeave(() => go);
    else go();
  };
  const newTask = () => {
    // A task in which nothing was said yet is the new task already.
    if (workspace.sessionId !== null && workspace.events.length === 0 && !running) field.current?.focus();
    else leave(() => void workspace.openSession());
  };
  // The service stops the turn of a task that is left and drops what waited in it, and it does
  // not tell of either: the conversation is with another task by then. What waited is kept
  // above, with the draft of the task.
  const openTask = (id: string) => leave(() => void workspace.openSession(id));
  const leaveProject = async (chosen: Promise<string | null>) => {
    const folder = await chosen;
    if (folder !== null) await workspace.openFolder(folder);
  };
  const openProject = () => leave(() => void leaveProject(workspace.pickFolder(shownLanguage(language))));
  const openRecent = (kept: string) => leave(() => void leaveProject(workspace.pickRecent(kept)));
  const openChat = () => leave(() => void workspace.openChat());

  // A page that the application itself leads to, such as one of GitHub. It is opened without a question.
  const openPage = (url: string) => {
    if (window.columbina) void window.columbina.openExternal(url).catch(workspace.fail);
    else window.open(url, '_blank', 'noopener,noreferrer');
  };

  // --- What a button under the end of a turn does. One press does it once.
  const acting = useRef(false);
  const act = async (give: () => Promise<boolean>) => {
    if (acting.current || workspace.sending || running || unavailable) return;
    acting.current = true;
    try {
      await give();
    } finally {
      acting.current = false;
    }
  };
  const onAction = (action: ActionKey, turn: Turn) => {
    if (action === 'openModelSettings') setSettings('model');
    else if (action === 'openPermissions') setSettings('permissions');
    else if (action === 'compact') void act(() => workspace.send('/compact'));
    // The task of a turn is given again as what it is: words for the agent. It is not looked
    // at for a command, whatever it begins with.
    else if (action === 'retry') void act(() => workspace.prompt(retryText(turn, words.prompt.goOn)));
  };

  // --- Telling the user of what waits for them while the window is in the background.
  const title = taskTitle(words, projection.items.find(
    (item): item is UserItem => item.type === 'user' && item.source === 'user')?.text ?? null);
  const asked = workspace.approval;
  useEffect(() => {
    if (asked === null || !tellMe || document.hasFocus()) return;
    void window.columbina?.notify({ title: words.label.notice.consent, body: asked.target ?? asked.summary }).catch(() => {});
    // Told once for each question. The words are those of the moment it came.
  }, [asked?.requestId]);
  const wasRunning = useRef(false);
  useEffect(() => {
    const ended = wasRunning.current && !running;
    wasRunning.current = running;
    if (!ended || !tellMe || document.hasFocus()) return;
    const last = projection.turns.at(-1)?.end;
    const well = last?.reason === 'completed';
    void window.columbina?.notify({ title: well ? words.label.notice.finished : words.label.notice.stopped, body: title })
      .catch(() => {});
  }, [running]);
  useEffect(() => {
    if (workspace.closed === null || !tellMe || document.hasFocus()) return;
    void window.columbina?.notify({ title: words.label.notice.stopped, body: goneWords(words, workspace.hello !== null) })
      .catch(() => {});
  }, [workspace.closed]);

  // --- The keyboard.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (showOpening || document.querySelector('dialog[open]')) return;
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey || event.repeat) return;
      const key = event.key.toLowerCase();
      if (key === 'b') {
        event.preventDefault();
        setSidebarHidden(hidden => !hidden);
      } else if (key === 'n' && workspace.connected && !workspace.loading) {
        event.preventDefault();
        newTask();
      } else if (key === ',') {
        event.preventDefault();
        setSettings('appearance');
      } else if (key === 'j' && traced) {
        event.preventDefault();
        showView(view === 'trace' ? 'conversation' : 'trace');
      } else if (key === 'k' && !unavailable) {
        event.preventDefault();
        setCommandsAsked(count => count + 1);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
  // The keyboard begins where the next thing is done, unless it is somewhere already.
  useEffect(() => {
    if (showOpening || document.querySelector('dialog[open]')) return;
    if (document.activeElement !== document.body && document.activeElement !== document.documentElement) return;
    if (!workspace.hasProject) firstButton.current?.focus();
    else if (!unavailable) field.current?.focus();
    // The field cannot be used and has let go of the keyboard. What is done next is a new start.
    else if (workspace.closed !== null || stopped) restartButton.current?.focus();
  }, [showOpening, unavailable, workspace.hasProject, workspace.sessionId, workspace.closed, stopped]);

  // --- What is shown.
  // A conversation in no project is called a chat, and not by the name of its folder.
  const folder = workspace.chat ? words.label.chat
    : workspace.hello ? folderName(workspace.hello.cwd)
      : workspace.folder ? folderName(workspace.folder) : '';
  const changes = changesOf(workspace).length;
  const writing = workspace.preview.text !== '' || workspace.preview.reasoning !== '' || workspace.preview.tools.length > 0;
  const empty = projection.items.length === 0 && projection.turns.length === 0
    && workspace.outputs.length === 0 && !writing && !running;
  // A long log is not shown page by page as it comes: the line that counts stands until all of it is there.
  const starting = workspace.hasProject && workspace.loading && (empty || workspace.reading !== null);
  // A page with nothing on it yet: the field stands in its middle.
  const bare = !workspace.hasProject || empty;
  // A service that ended with nothing on the page: there is nothing to welcome anyone to,
  // and the line that says so stands alone.
  const gone = workspace.hasProject && workspace.closed !== null && empty;
  // A task with something to show can be shown as its conversation or as its trace.
  const traced = workspace.hasProject && !bare && !starting && !gone && workspace.sessionId !== null;
  const view: TaskView = traced ? views[workspace.sessionId!] ?? 'conversation' : 'conversation';
  const showView = (next: TaskView) => {
    const id = workspace.sessionId;
    if (id !== null) setViews(current => ({ ...current, [id]: next }));
  };
  // Without a project there are no tasks to list.
  const noSidebar = sidebarHidden || !workspace.hasProject;
  const showInspector = (tab: InspectorTab) => setInspector(current => (current === tab ? null : tab));
  // The name of the window, as the system shows it where windows are listed.
  useEffect(() => {
    document.title = folder ? `${folder} · Columbina` : 'Columbina';
  }, [folder]);

  return <Words.Provider value={words}>
    <div
      className="window"
      data-native={window.columbina ? window.columbina.platform ?? '' : undefined}
      data-full-screen={fullScreen ? '' : undefined}
    >
      <header className="titlebar">
        <span className="brand"><span className="brand-moon" aria-hidden="true" />Columbina</span>
        {folder && <span className="titlebar-project" title={workspace.hello?.cwd}>{folder}</span>}
        <MusicTitlebar music={music} />
        {/* How the task that is open is shown: as its conversation, or as its trace. Not over the opening. */}
        {traced && !showOpening && <div className="titlebar-views" role="group" aria-label={words.label.trace.views} data-test="views">
          {(['conversation', 'trace'] as const).map(each => <button
            key={each}
            type="button"
            aria-pressed={view === each}
            title={`${words.label.trace.view[each]} (${shortcut('J')})`}
            onClick={() => showView(each)}
            data-test={`view-${each}`}
          >
            {each === 'conversation' ? <MessageSquare size={13} aria-hidden="true" /> : <ListTree size={13} aria-hidden="true" />}
            <span>{words.label.trace.view[each]}</span>
          </button>)}
        </div>}
        {/* A newer version, at the end of the bar: a click opens the page of its release. Not over the opening. */}
        {told && !showOpening && <button
          type="button"
          className="titlebar-update"
          aria-label={words.label.updates.toldInFull(told.version)}
          title={words.label.updates.toldInFull(told.version)}
          onClick={() => openPage(told.page)}
          data-test="update-told"
        >
          <Download size={13} aria-hidden="true" /><span>{words.label.updates.told(told.version)}</span>
        </button>}
      </header>

      {showOpening && <StartupVideo
        hasBackgroundMusic={music.hasTrack}
        muted={music.muted}
        onMutedChange={music.setMuted}
        onComplete={() => setShowOpening(false)}
      />}

      <div
        className="shell"
        inert={showOpening}
        aria-hidden={showOpening || undefined}
        data-sidebar={noSidebar ? 'hidden' : undefined}
        data-inspector={inspector ? '' : undefined}
      >
        <a href="#composer" className="skip">{words.label.skipToInput}</a>

        {!noSidebar && <Sidebar
          workspace={workspace}
          title={title}
          onNewTask={newTask}
          onOpenTask={openTask}
          onOpenProject={openProject}
          onOpenChat={openChat}
          onSettings={() => setSettings('appearance')}
          onHide={() => setSidebarHidden(true)}
        />}

        <main className="main">
          <header className="topbar">
            {sidebarHidden && workspace.hasProject && <button
              type="button"
              className="icon-btn"
              onClick={() => setSidebarHidden(false)}
              aria-label={words.label.showTasks}
              title={`${words.label.showTasks} (${shortcut('B')})`}
            >
              <PanelLeft size={16} />
            </button>}
            {/* An empty page says what it is in its middle, and the panel has these two ways into it itself. */}
            <h1 className="topbar-title" title={title} data-test="title">{bare ? '' : title}</h1>
            {!workspace.hasProject && <button
              type="button"
              className="icon-btn"
              onClick={() => setSettings('appearance')}
              aria-label={words.label.settings}
              title={words.label.settings}
              data-test="open-settings"
            >
              <Settings2 size={16} />
            </button>}
            {!bare && !inspector && <div className="topbar-tools">
              <button type="button" className="btn btn-quiet" aria-pressed={inspector === 'changes'} onClick={() => showInspector('changes')} data-test="show-changes">
                <FileDiff size={15} aria-hidden="true" />{words.label.changes}
                {changes > 0 && <span className="count">{changes}</span>}
              </button>
              <button
                type="button"
                className="icon-btn"
                aria-pressed={inspector === 'details'}
                aria-label={words.label.details}
                title={words.label.details}
                onClick={() => showInspector('details')}
                data-test="show-details"
              >
                <SlidersHorizontal size={16} />
              </button>
            </div>}
          </header>

          {workspace.closed !== null && <div className="banner" data-tone="bad" role="alert" data-test="banner-closed">
            <div className="banner-words">
              <p data-test="closed-what">{goneWords(words, workspace.hello !== null)}</p>
              {/* Why is most often in what it wrote last, so that is not put away. */}
              {workspace.lastWords.length > 0 && <>
                <div className="overline last-words-title">{words.label.lastWords}</div>
                <pre className="block last-words" tabIndex={0} data-test="last-words">{workspace.lastWords.join('\n')}</pre>
              </>}
              {workspace.closed && <details className="aside">
                <summary><ChevronRight size={13} aria-hidden="true" />{words.label.details}</summary>
                <p className="tool-code mono">{workspace.closed}</p>
              </details>}
            </div>
            <div className="banner-actions">
              {window.columbina && <button type="button" className="btn btn-quiet" onClick={workspace.showLog} data-test="show-log">
                <FileText size={14} aria-hidden="true" />{words.label.showLog}
              </button>}
              <button type="button" ref={restartButton} className="btn" disabled={workspace.loading} onClick={() => void workspace.reconnect()} data-test="restart">
                {words.label.restart}
              </button>
            </div>
          </div>}
          {stopped && workspace.closed === null && <div className="banner" data-tone="bad" role="alert" data-test="banner-stopped">
            <div className="banner-words">
              <p>{words.hint.taskStopped}</p>
              {status?.detail && <details className="aside">
                <summary><ChevronRight size={13} aria-hidden="true" />{words.label.details}</summary>
                <p className="tool-code mono">{status.detail}</p>
              </details>}
            </div>
            <button type="button" ref={restartButton} className="btn" disabled={workspace.loading} onClick={() => void workspace.reconnect()} data-test="restart">
              {words.label.restart}
            </button>
          </div>}
          {/* What failed because the service is gone is said by the line above. */}
          {workspace.error && workspace.closed === null && <div className="banner" data-tone="warn" role="alert" data-test="banner-error">
            <div className="banner-words">
              <p>{words.hint.requestFailed}</p>
              <p className="caption">{workspace.error}</p>
            </div>
            <button type="button" className="icon-btn" onClick={workspace.clearError} aria-label={words.label.close} title={words.label.close}>
              <X size={15} />
            </button>
          </div>}

          {/* The field keeps its place among these, so that it keeps the keyboard when the page changes. */}
          <div className="stage" data-empty={bare ? '' : undefined}>
            {starting
              ? <Starting workspace={workspace} />
              : gone ? null
                : bare
                  ? <StartHead
                      workspace={workspace}
                      github={github}
                      onOpenProject={openProject}
                      onOpenRecent={openRecent}
                      onOpenChat={openChat}
                      onPermissions={() => setSettings('permissions')}
                      onSetUp={service => void workspace.setUpService(service, shownLanguage(language))}
                      onOpenPage={openPage}
                      firstButton={firstButton}
                    />
                  : view === 'trace'
                    ? <TraceView workspace={workspace} locale={shownLanguage(language)} />
                    : <Conversation workspace={workspace} onLink={setLink} onAction={onAction} />}
            <div className="dock" hidden={!workspace.hasProject || starting || gone}>
              {/* A card of its own for every question, so that nothing of one question is left for the next. */}
              {workspace.approval && <ApprovalCard
                key={workspace.approval.requestId}
                approval={workspace.approval}
                cancelling={workspace.cancelling}
                onAnswer={(outcome, remember) => {
                  workspace.answer(workspace.approval!.requestId, outcome, remember);
                  if (outcome === 'rejected') setDeclined(true);
                }}
                onStop={() => void workspace.cancel()}
              />}
              <Composer
                workspace={workspace}
                draft={draft}
                onDraft={write}
                onSend={sendDraft}
                field={field}
                returned={returned}
                declined={declined}
                commandsAsked={commandsAsked}
                models={models}
                onChooseModel={model => void chooseModel(model)}
                onSwitchService={(service, model) => void switchService(service, model)}
                onChooseEffort={effort => void chooseEffort(effort)}
                onModelSettings={() => setSettings('model')}
                attachments={attached}
                refused={refused}
                onAttach={canAttach ? () => void chooseAttachments() : undefined}
                onDropFiles={canAttach ? files => void dropAttachments(files) : undefined}
                onRemoveAttachment={removeAttachment}
              />
            </div>
            {bare && !starting && !gone && workspace.hasProject && <>
              <Recent workspace={workspace} onOpenTask={openTask} />
              <StartSkills
                workspace={workspace}
                onUse={(name) => {
                  // The command that hands the skill over, with the field ready for the task to do by it.
                  write(skillLine(name));
                  field.current?.focus();
                }}
              />
            </>}
          </div>

          <StatusLine
            workspace={workspace}
            locale={shownLanguage(language)}
            onPermissions={() => setSettings('permissions')}
            onDetails={() => setInspector('details')}
          />
        </main>

        {inspector && <Inspector
          workspace={workspace}
          tab={inspector}
          onTab={setInspector}
          onClose={() => setInspector(null)}
          locale={shownLanguage(language)}
        />}
      </div>

      {settings && <Settings
        workspace={workspace}
        part={settings}
        theme={theme}
        onTheme={setTheme}
        size={size}
        onSize={setSize}
        language={language}
        onLanguage={setLanguage}
        shownLanguage={shownLanguage(language)}
        playOpening={playOpening}
        onPlayOpening={setPlayOpening}
        tellMe={tellMe}
        onTellMe={setTellMe}
        onPlayNow={() => { setSettings(null); setShowOpening(true); }}
        music={music}
        github={github}
        models={models}
        updates={updates}
        checkingUpdates={checkingUpdates}
        onCheckingUpdates={setCheckingUpdates}
        skipped={skipped}
        onSkip={skip}
        onOpen={openPage}
        onChooseModel={model => void chooseModel(model)}
        onSwitchService={(service, model) => void switchService(service, model)}
        onChooseEffort={effort => void chooseEffort(effort)}
        // What is changed here shows here: the answer of the command is not written into the conversation.
        onCommand={line => void send(line, true)}
        notKept={notKept}
        onClose={() => { setSettings(null); setNotKept(null); }}
      />}

      {pendingPermission && <Sheet
        title={pendingPermission.preset === 'full-access' ? words.label.confirmFullAccess : words.label.confirmWrite}
        onClose={() => setPendingPermission(null)}
        foot={<>
          <button type="button" className="btn" data-first onClick={() => setPendingPermission(null)}>{words.label.cancel}</button>
          <button
            type="button"
            className={pendingPermission.preset === 'full-access' ? 'btn btn-bad' : 'btn btn-main'}
            onClick={() => void confirmPermission()}
            data-test="confirm-permission"
          >
            {pendingPermission.preset === 'full-access' ? words.label.allowFullAccess : words.label.allowWrite}
          </button>
        </>}
      >
        <p>{pendingPermission.preset === 'full-access' ? words.hint.confirmFullAccess : words.hint.confirmWrite}</p>
        {workspace.hello && <p className="caption mono">{workspace.hello.cwd}</p>}
      </Sheet>}

      {pendingLeave && <Sheet
        title={words.label.confirmLeave}
        onClose={() => setPendingLeave(null)}
        foot={<>
          <button type="button" className="btn" data-first onClick={() => setPendingLeave(null)}>{words.label.stay}</button>
          <button
            type="button"
            className="btn btn-bad"
            onClick={() => { const go = pendingLeave; setPendingLeave(null); go(); }}
            data-test="confirm-leave"
          >
            {words.label.stopAndLeave}
          </button>
        </>}
      >
        <p>{words.hint.confirmLeave}</p>
      </Sheet>}

      {link && <Sheet
        title={words.label.openLinkTitle}
        onClose={() => setLink(null)}
        foot={<>
          <button type="button" className="btn" data-first onClick={() => setLink(null)}>{words.label.cancel}</button>
          <button
            type="button"
            className="btn btn-main"
            // Only an address of the web is handed on. The shell checks the same.
            disabled={!/^https?:\/\//i.test(link)}
            onClick={() => {
              const address = link;
              setLink(null);
              if (window.columbina) void window.columbina.openExternal(address).catch(workspace.fail);
              else window.open(address, '_blank', 'noopener,noreferrer');
            }}
          >
            {words.label.openLink}
          </button>
        </>}
      >
        <p>{words.hint.openLink}</p>
        <pre className="block" tabIndex={0}>{link}</pre>
      </Sheet>}
    </div>
  </Words.Provider>;
}
