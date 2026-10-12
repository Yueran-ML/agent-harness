import { useEffect, useMemo, useRef, useState } from 'react';
import { ChevronRight, FileDiff, FileText, PanelLeft, Settings2, SlidersHorizontal, X } from 'lucide-react';
import { MusicTitlebar, useBackgroundMusic } from '../BackgroundMusic';
import { StartupVideo } from '../StartupVideo';
import type { ActionKey } from '../outcomes';
import { messageText, type UserItem } from '../state';
import { initialTheme, storeTheme, type ThemeId } from '../themes';
import type { Turn } from '../turns';
import {
  canSubmitInput, permissionNeedsConfirmation, permissionPresetFromCommand, type PermissionPreset,
} from '../uiActions';
import { useWorkspace, type Workspace } from '../useWorkspace';
import { ApprovalCard } from './ApprovalCard';
import { Composer } from './Composer';
import { Conversation } from './Conversation';
import { changesOf, Inspector, type InspectorTab } from './Inspector';
import { Settings, type SettingsPart } from './Settings';
import { Sheet } from './Sheet';
import { Sidebar } from './Sidebar';
import { Recent, StartHead } from './Start';
import { StatusLine } from './StatusLine';
import { Drafts, samePlace, type Place } from './drafts';
import { shownLanguage, storedLanguage, storeLanguage, useWords, Words, wordsFor, type LanguageChoice } from './locale';
import { storedOpening, storeOpening } from './opening';
import { folderName, goneWords, joined, retryText, STILL_STARTING_MS, taskTitle, waitingWords } from './tell';

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
}

export default function App() {
  const workspace = useWorkspace();
  const music = useBackgroundMusic();
  const { projection, status } = workspace;

  const [theme, setTheme] = useState<ThemeId>(initialTheme);
  const [language, setLanguage] = useState<LanguageChoice>(storedLanguage);
  const words = useMemo(() => wordsFor(language), [language]);
  // The opening plays at the first start. After that it plays where the user chose to see it
  // at every start, which is what the switch in the settings says.
  const [opening] = useState(storedOpening);
  const [playOpening, setPlayOpening] = useState(opening.always);
  const [showOpening, setShowOpening] = useState(opening.show);
  const [tellMe, setTellMe] = useState(storedTelling);

  const [sidebarHidden, setSidebarHidden] = useState(false);
  const [inspector, setInspector] = useState<InspectorTab | null>(null);
  const [settings, setSettings] = useState<SettingsPart | null>(null);
  const [link, setLink] = useState<string | null>(null);
  const [pendingPermission, setPendingPermission] = useState<PendingPermission | null>(null);
  // Something that would stop the turn that is under way, and waits for a yes.
  const [pendingLeave, setPendingLeave] = useState<(() => void) | null>(null);
  // How often the list of commands was asked for with the keyboard.
  const [commandsAsked, setCommandsAsked] = useState(0);

  const field = useRef<HTMLTextAreaElement>(null);
  const openButton = useRef<HTMLButtonElement>(null);

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
    // The language that is shown, which is English where the system has one that there are no words for.
    document.documentElement.lang = shownLanguage(language);
    storeLanguage(language);
  }, [language]);
  useEffect(() => {
    try {
      localStorage.setItem(tellingKey, String(tellMe));
    } catch {
      // Without storage the choice holds for this window.
    }
  }, [tellMe]);
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
  const project = workspace.hello?.cwd ?? null;
  const place = useMemo<Place | null>(
    () => (workspace.sessionId === null ? null : { project, task: workspace.sessionId }),
    [project, workspace.sessionId],
  );
  const placeNow = useRef<Place | null>(null);
  useEffect(() => {
    if (placeNow.current !== null) drafts.current.keep(placeNow.current, draftNow.current);
    placeNow.current = place;
    setDraft(place === null ? '' : drafts.current.of(place));
    setReturned(false);
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
  const send = async (line: string): Promise<boolean> => {
    if (!canSubmitInput(line, state)) return false;
    const preset = permissionPresetFromCommand(line);
    if (preset && permissionNeedsConfirmation(preset, status)) {
      setPendingPermission({ preset, line, sessionId: workspace.sessionId });
      return false;
    }
    return workspace.send(line);
  };
  /** What was sent from a task is no draft of it any more, wherever the user is by the time the answer comes. */
  const forget = (from: Place | null, line: string) => {
    if (from === null) return;
    if (samePlace(from, placeNow.current)) setDraft(current => (current === line ? '' : current));
    else drafts.current.sent(from, line);
  };
  const sendDraft = async (line: string): Promise<boolean> => {
    const from = placeNow.current;
    const taken = await send(line);
    if (taken) forget(from, line);
    return taken;
  };
  const confirmPermission = async () => {
    const change = pendingPermission;
    setPendingPermission(null);
    if (!change || change.sessionId !== workspace.sessionId || !canSubmitInput(change.line, state)) return;
    const from = placeNow.current;
    if (await workspace.send(change.line)) forget(from, change.line);
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
  // The service stops the turn of the task that is left, and tells of what it dropped before it
  // answers. So what was not read is back in the field by the time the task is left, and stays
  // with it as its draft.
  const openTask = (id: string) => leave(() => void workspace.openSession(id));
  // A project is left without a word from its service. What waits there unread is kept as a draft.
  const unread = useRef<string[]>([]);
  unread.current = [...projection.queues['next-turn'], ...projection.queues['next-step']]
    .filter(entry => entry.source === 'user' || entry.source === 'steer')
    .map(entry => messageText(entry.message));
  const leaveProject = async (chosen: Promise<string | null>) => {
    const folder = await chosen;
    if (folder === null) return;
    // It goes into the field, and from there into the draft of the task as the task is left.
    if (unread.current.length > 0) setDraft(current => joined([...unread.current, current]));
    await workspace.openFolder(folder);
  };
  const openProject = () => leave(() => void leaveProject(workspace.pickFolder(shownLanguage(language))));
  const openRecent = (kept: string) => leave(() => void leaveProject(workspace.pickRecent(kept)));

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
    void window.columbina?.notify({ title: words.label.notice.stopped, body: words.hint.serviceEnded }).catch(() => {});
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
    if (!workspace.hasProject) openButton.current?.focus();
    else if (!unavailable) field.current?.focus();
  }, [showOpening, unavailable, workspace.hasProject, workspace.sessionId]);

  // --- What is shown.
  const folder = workspace.hello ? folderName(workspace.hello.cwd)
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
  // Without a project there are no tasks to list.
  const noSidebar = sidebarHidden || !workspace.hasProject;
  const showInspector = (tab: InspectorTab) => setInspector(current => (current === tab ? null : tab));
  // The name of the window, as the system shows it where windows are listed.
  useEffect(() => {
    document.title = folder ? `${folder} · Columbina` : 'Columbina';
  }, [folder]);

  return <Words.Provider value={words}>
    <div className="window" data-native={window.columbina ? '' : undefined}>
      <header className="titlebar">
        <span className="brand"><span className="brand-moon" aria-hidden="true" />Columbina</span>
        {folder && <span className="titlebar-project" title={workspace.hello?.cwd}>{folder}</span>}
        <MusicTitlebar music={music} />
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
              title={`${words.label.showTasks} (Ctrl+B)`}
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
              <button type="button" className="btn" disabled={workspace.loading} onClick={() => void workspace.reconnect()} data-test="restart">
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
            <button type="button" className="btn" disabled={workspace.loading} onClick={() => void workspace.reconnect()} data-test="restart">
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
                  ? <StartHead workspace={workspace} onOpenProject={openProject} onOpenRecent={openRecent} openButton={openButton} />
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
              />
            </div>
            {bare && !starting && !gone && workspace.hasProject && <Recent workspace={workspace} onOpenTask={openTask} />}
          </div>

          <StatusLine
            workspace={workspace}
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
        language={language}
        onLanguage={setLanguage}
        shownLanguage={shownLanguage(language)}
        playOpening={playOpening}
        onPlayOpening={setPlayOpening}
        tellMe={tellMe}
        onTellMe={setTellMe}
        onPlayNow={() => { setSettings(null); setShowOpening(true); }}
        music={music}
        onCommand={line => void send(line)}
        onClose={() => setSettings(null)}
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
