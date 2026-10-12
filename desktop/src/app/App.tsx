import { useEffect, useMemo, useRef, useState } from 'react';
import { ChevronRight, FileDiff, PanelLeft, Settings2, SlidersHorizontal, X } from 'lucide-react';
import { MusicTitlebar, useBackgroundMusic } from '../BackgroundMusic';
import { StartupVideo } from '../StartupVideo';
import type { ActionKey } from '../outcomes';
import { messageText } from '../state';
import { initialTheme, storeTheme, type ThemeId } from '../themes';
import { promptOf, type Turn } from '../turns';
import {
  canSubmitInput, permissionNeedsConfirmation, permissionPresetFromCommand, type PermissionPreset,
} from '../uiActions';
import { useWorkspace } from '../useWorkspace';
import { ApprovalCard } from './ApprovalCard';
import { Composer } from './Composer';
import { Conversation } from './Conversation';
import { changesOf, Inspector, type InspectorTab } from './Inspector';
import { Settings, type SettingsPart } from './Settings';
import { Sheet } from './Sheet';
import { Sidebar } from './Sidebar';
import { Recent, StartHead } from './Start';
import { StatusLine } from './StatusLine';
import { localeOf, storedLanguage, storeLanguage, Words, wordsFor, type LanguageChoice } from './locale';
import { folderName, taskTitle } from './tell';

const openingKey = 'columbina-play-intro';

function storedOpening(): boolean {
  try {
    return localStorage.getItem(openingKey) !== 'false';
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
  const [playOpening, setPlayOpening] = useState(storedOpening);
  const [showOpening, setShowOpening] = useState(playOpening);

  const [sidebarHidden, setSidebarHidden] = useState(false);
  const [inspector, setInspector] = useState<InspectorTab | null>(null);
  const [settings, setSettings] = useState<SettingsPart | null>(null);
  const [link, setLink] = useState<string | null>(null);
  const [pendingPermission, setPendingPermission] = useState<PendingPermission | null>(null);
  // Something that would stop the turn that is under way, and waits for a yes.
  const [pendingLeave, setPendingLeave] = useState<(() => void) | null>(null);

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
    document.documentElement.lang = localeOf(language);
    storeLanguage(language);
  }, [language]);
  useEffect(() => {
    try {
      localStorage.setItem(openingKey, String(playOpening));
    } catch {
      // Without storage the choice holds for this window.
    }
  }, [playOpening]);
  // A question of the agent is not kept waiting behind a film.
  useEffect(() => { if (workspace.approval) setShowOpening(false); }, [workspace.approval]);

  // --- What is written and not sent. Every task keeps its own.
  const [draft, setDraft] = useState('');
  const [returned, setReturned] = useState(false);
  const drafts = useRef(new Map<string, string>());
  const draftNow = useRef(draft);
  draftNow.current = draft;
  const draftOf = useRef<string | null>(null);
  useEffect(() => {
    if (draftOf.current !== null) drafts.current.set(draftOf.current, draftNow.current);
    draftOf.current = workspace.sessionId;
    setDraft(workspace.sessionId === null ? '' : drafts.current.get(workspace.sessionId) ?? '');
    setReturned(false);
    setPendingPermission(null);
  }, [workspace.sessionId]);
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
    setDraft(current => [...fresh.map(input => input.text), current].filter(text => text.trim() !== '').join('\n'));
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
  const sendDraft = async (line: string): Promise<boolean> => {
    const taken = await send(line);
    // What was written meanwhile is not wiped away.
    if (taken) setDraft(current => (current === line ? '' : current));
    return taken;
  };
  const confirmPermission = async () => {
    const change = pendingPermission;
    setPendingPermission(null);
    if (!change || change.sessionId !== workspace.sessionId || !canSubmitInput(change.line, state)) return;
    if (await workspace.send(change.line)) setDraft(current => (current.trim() === change.line ? '' : current));
  };

  // --- Leaving a task. A turn that is under way is not stopped without a yes.
  const leaving = (go: () => void) => () => {
    // What still waits to be read would be lost with the turn. It stays with the task, as a draft.
    const unread = [...projection.queues['next-turn'], ...projection.queues['next-step']]
      .filter(entry => entry.source === 'user' || entry.source === 'steer')
      .map(entry => messageText(entry.message));
    if (unread.length > 0) setDraft(current => [...unread, current].filter(text => text.trim() !== '').join('\n'));
    go();
  };
  const leave = (go: () => void) => {
    if (running) setPendingLeave(() => leaving(go));
    else go();
  };
  const newTask = () => {
    // A task in which nothing was said yet is the new task already.
    if (workspace.sessionId !== null && workspace.events.length === 0 && !running) field.current?.focus();
    else leave(() => void workspace.openSession());
  };
  const openTask = (id: string) => leave(() => void workspace.openSession(id));
  const openProject = () => leave(() => void workspace.chooseFolder());

  // --- What a button under the end of a turn does.
  const onAction = (action: ActionKey, turn: Turn) => {
    if (action === 'openModelSettings') setSettings('model');
    else if (action === 'openPermissions') setSettings('permissions');
    else if (action === 'compact') void workspace.send('/compact');
    else if (action === 'retry') {
      // A turn that had done nothing yet is given its task again. One that had begun goes on.
      const prompt = promptOf(turn);
      const begun = turn.steps.some(item => item.type === 'tool' || item.type === 'assistant');
      void workspace.send(begun || prompt === null ? words.prompt.goOn : prompt.text);
    }
  };

  // --- The keyboard.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (showOpening || document.querySelector('dialog[open]')) return;
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey) return;
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
        if (draftNow.current.trim() === '' && !running) write('/');
        field.current?.focus();
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
  const first = projection.items.find(item => item.type === 'user' && item.source === 'user');
  const title = first?.type === 'user' ? taskTitle(words, first.text) : words.label.newTask;
  const project = workspace.hello ? folderName(workspace.hello.cwd)
    : workspace.folder ? folderName(workspace.folder) : '';
  const changes = changesOf(workspace).length;
  const writing = workspace.preview.text !== '' || workspace.preview.reasoning !== '' || workspace.preview.tools.length > 0;
  const empty = projection.items.length === 0 && projection.turns.length === 0
    && workspace.outputs.length === 0 && !writing && !running;
  const starting = workspace.hasProject && workspace.loading && empty;
  // A page with nothing on it yet: the field stands in its middle.
  const bare = !workspace.hasProject || empty;
  // Without a project there are no tasks to list.
  const noSidebar = sidebarHidden || !workspace.hasProject;
  const showInspector = (tab: InspectorTab) => setInspector(current => (current === tab ? null : tab));
  // The name of the window, as the system shows it where windows are listed.
  useEffect(() => {
    document.title = project ? `${project} · Columbina` : 'Columbina';
  }, [project]);

  return <Words.Provider value={words}>
    <div className="window" data-native={window.columbina ? '' : undefined}>
      <header className="titlebar">
        <span className="brand"><span className="brand-moon" aria-hidden="true" />Columbina</span>
        {project && <span className="titlebar-project" title={workspace.hello?.cwd}>{project}</span>}
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
              <p>{words.hint.serviceEnded}</p>
              {workspace.closed && <details className="aside">
                <summary><ChevronRight size={13} aria-hidden="true" />{words.label.details}</summary>
                <p className="tool-code mono">{workspace.closed}</p>
              </details>}
            </div>
            <button type="button" className="btn" disabled={workspace.loading} onClick={() => void workspace.reconnect()}>
              {words.label.restart}
            </button>
          </div>}
          {stopped && workspace.closed === null && <div className="banner" data-tone="bad" role="alert" data-test="banner-stopped">
            <div className="banner-words">
              <p>{words.hint.taskStopped}</p>
              {status?.detail && <details className="aside">
                <summary><ChevronRight size={13} aria-hidden="true" />{words.label.details}</summary>
                <p className="tool-code mono">{status.detail}</p>
              </details>}
            </div>
            <button type="button" className="btn" disabled={workspace.loading} onClick={() => void workspace.reconnect()}>
              {words.label.restart}
            </button>
          </div>}
          {workspace.error && <div className="banner" data-tone="warn" role="alert" data-test="banner-error">
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
              ? <p className="working" role="status" data-test="starting">
                <span className="moon" aria-hidden="true" />
                {workspace.hello || !workspace.folder ? words.hint.openingTask : words.hint.starting(workspace.folder)}
              </p>
              : bare
                ? <StartHead workspace={workspace} onOpenProject={openProject} openButton={openButton} />
                : <Conversation workspace={workspace} onLink={setLink} onAction={onAction} />}
            <div className="dock" hidden={!workspace.hasProject || starting}>
              {workspace.approval && <ApprovalCard
                approval={workspace.approval}
                cancelling={workspace.cancelling}
                onAnswer={(outcome, remember) => {
                  workspace.answer(outcome, remember);
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
              />
            </div>
            {bare && !starting && workspace.hasProject && <Recent workspace={workspace} onOpenTask={openTask} />}
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
          locale={localeOf(language)}
        />}
      </div>

      {settings && <Settings
        workspace={workspace}
        part={settings}
        theme={theme}
        onTheme={setTheme}
        language={language}
        onLanguage={setLanguage}
        playOpening={playOpening}
        onPlayOpening={setPlayOpening}
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
