import { useEffect, useRef, useState, type FormEvent } from 'react';
import { ArrowDownToLine, ArrowUp, ArrowUpRight, Check, ChevronDown, ChevronRight, CircleHelp, Command, Folder, FolderOpen, Gauge, Info, LoaderCircle, MessageSquare, Moon, Search, Settings2, ShieldCheck, ShieldOff, SlidersHorizontal, Sparkles, Square, Sun, Terminal, WifiOff, X, PanelLeft, FileDiff, SquarePen } from 'lucide-react';
import { EditHistory } from './EditHistory';
import { Conversation } from './Conversation';
import { Dialog } from './Dialog';
import { StartupVideo } from './StartupVideo';
import { MusicSettings, MusicTitlebar, useBackgroundMusic } from './BackgroundMusic';
import { initialTheme, themes, themeStorageKey, type ThemeId } from './themes';
import { useWorkspace, type Workspace } from './useWorkspace';
import { canSubmitInput, permissionNeedsConfirmation, permissionPresetFromCommand, type PermissionPreset } from './uiActions';

function basename(path: string) { return path.split(/[\\/]/).filter(Boolean).at(-1) || 'Workspace'; }
function compact(value: number) { return Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 }).format(value); }

function Details({ workspace }: { workspace: Workspace }) {
  const { projection: p, status } = workspace;
  const input = p.usage.input_tokens + p.usage.cache_read_tokens + p.usage.cache_write_tokens;
  const cache = input ? Math.round(p.usage.cache_read_tokens / input * 100) : 0;
  const context = status?.model.context_window ? Math.min(100, Math.round(p.lastInputTokens / status.model.context_window * 100)) : null;
  return <div className="details-body">
    <div className="details-section"><span className="eyebrow">CURRENT MODEL</span><h3>{status?.model.model || 'Connecting'}</h3><p>{status?.model.provider || 'Waiting for the service'}</p><span className={`status-label ${status?.status === 'running' ? 'running' : ''}`}><span />{status?.status ?? 'Connecting'}</span></div>
    <div className="details-section"><div className="section-title"><span>Context window</span><strong>{context === null ? '—' : `${context}%`}</strong></div><div className="meter"><span style={{ width: `${context ?? 0}%` }} /></div><p>{compact(p.lastInputTokens)} / {status?.model.context_window ? compact(status.model.context_window) : 'unknown'} tokens · last request</p>
      <div className="usage-grid"><div><strong>{compact(input)}</strong><span>Input tokens</span></div><div><strong>{compact(p.usage.output_tokens)}</strong><span>Output tokens</span></div><div><strong>{cache}%</strong><span>Cache hit rate</span></div><div><strong>{p.items.filter(item => item.type === 'tool').length}</strong><span>Tool calls</span></div></div>
    </div>
    <div className="details-section"><span className="eyebrow">AVAILABLE TOOLS</span>{p.tools.length ? <div className="tools-list">{p.tools.map(tool => <details key={tool.name}><summary><Terminal size={13} /><code>{tool.name}</code><ChevronRight size={12} /></summary><p>{tool.description}</p></details>)}</div> : <p>Tools appear when the first request begins.</p>}</div>
    <div className="details-section"><span className="eyebrow">SESSION</span><code className="session-id">{workspace.sessionId ?? 'Not open'}</code><p>{workspace.events.length} recorded events</p></div>
  </div>;
}

function Appearance({ theme, onTheme }: { theme: ThemeId; onTheme: (theme: ThemeId) => void }) {
  return <div className="appearance"><div className="settings-section-heading"><h3>Appearance</h3><p>Choose a theme for this application.</p></div><div className="theme-grid">{themes.map(item => <button key={item.id} className={`theme-choice ${theme === item.id ? 'selected' : ''}`} onClick={() => onTheme(item.id)} aria-pressed={theme === item.id}>
    <span className="theme-preview" style={{ background: item.swatches[0] }}><span className="preview-sidebar" style={{ background: item.swatches[1] }} /><span className="preview-lines"><i style={{ background: item.swatches[2] }} /><i style={{ background: item.swatches[2] }} /><i style={{ background: item.swatches[2] }} /></span><span className="preview-input" style={{ background: item.swatches[1], borderColor: item.swatches[2] }} /></span>
    <span className="theme-name">{item.name}{theme === item.id && <Check size={14} />}</span><small>{item.subtitle}</small>
  </button>)}</div><p className="theme-footnote">Nod-Krai pairs moonwhite surfaces with clear blue accents. Moonveil and Silvermoon offer darker evening palettes.</p></div>;
}

export default function App() {
  const workspace = useWorkspace();
  const music = useBackgroundMusic();
  const [theme, setTheme] = useState<ThemeId>(initialTheme);
  const [playIntro, setPlayIntro] = useState(() => { try { return localStorage.getItem('columbina-play-intro') !== 'false'; } catch { return true; } });
  const [showIntro, setShowIntro] = useState(playIntro);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [panelTab, setPanelTab] = useState<'edits' | 'details'>('edits');
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(248);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [link, setLink] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [draft, setDraft] = useState('');
  const [modelDraft, setModelDraft] = useState('');
  const [answering, setAnswering] = useState(false);
  const [pendingPermission, setPendingPermission] = useState<{
    preset: PermissionPreset; line: string; sessionId: string | null; fromComposer: boolean;
  } | null>(null);
  const textarea = useRef<HTMLTextAreaElement>(null);
  const openProjectButton = useRef<HTMLButtonElement>(null);
  const permissionCancel = useRef<HTMLButtonElement>(null);
  const submitInFlight = useRef(false);
  const running = workspace.status?.status === 'running';
  const stopped = workspace.status?.status === 'stopped';
  const unavailable = workspace.loading || !workspace.connected || stopped;
  const firstInput = workspace.projection.items.find(item => item.type === 'user' && item.source === 'user');
  const title = firstInput?.type === 'user' ? firstInput.text.split('\n')[0] : 'New conversation';
  const allCommands = workspace.hello?.commands ?? [];
  const commands = allCommands.filter(item => `/${item.name}`.startsWith(draft.split(' ')[0].toLowerCase()));
  const showCommands = draft.startsWith('/') && !draft.includes(' ') && commands.length > 0;
  const canModel = allCommands.some(item => item.name === 'model');
  const canPermissions = allCommands.some(item => item.name === 'permissions');
  const dangerous = workspace.status?.sandbox?.mode === 'danger-full-access' || workspace.status?.approval === 'never';
  const submitState = { unavailable, sending: workspace.sending, running };
  const canSubmit = canSubmitInput(draft, submitState);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    void window.columbina?.setTheme(theme).catch(workspace.fail);
    try { localStorage.setItem(themeStorageKey, theme); } catch { /* A window-local theme works when storage is unavailable. */ }
  }, [theme]);
  useEffect(() => setModelDraft(workspace.status?.model.model ?? ''), [workspace.status?.model.model]);
  useEffect(() => { setAnswering(false); if (workspace.approval) setShowIntro(false); }, [workspace.approval?.requestId]);
  useEffect(() => { try { localStorage.setItem('columbina-play-intro', String(playIntro)); } catch { /* Keep the window preference when storage is unavailable. */ } }, [playIntro]);
  useEffect(() => { setPendingPermission(null); }, [workspace.sessionId]);
  useEffect(() => { if (pendingPermission) permissionCancel.current?.focus(); }, [pendingPermission]);
  useEffect(() => {
    if (showIntro || document.querySelector('dialog[open]')) return;
    if (document.activeElement !== document.body && document.activeElement !== document.documentElement) return;
    if (!workspace.hasProject) openProjectButton.current?.focus();
    else if (!unavailable) textarea.current?.focus();
  }, [showIntro, unavailable, workspace.hasProject, workspace.sessionId]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (showIntro || document.querySelector('dialog[open]')) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); setPaletteOpen(current => !current); }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'b') { event.preventDefault(); setSidebarCollapsed(current => !current); }
      if (event.key === 'Escape' && !document.querySelector('dialog[open]')) { setSidebarOpen(false); setDetailsOpen(false); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [showIntro]);
  useEffect(() => {
    if (textarea.current) { textarea.current.style.height = 'auto'; textarea.current.style.height = `${Math.min(180, Math.max(52, textarea.current.scrollHeight))}px`; }
  }, [draft]);

  const suggest = (text: string) => { setDraft(text); textarea.current?.focus(); };
  const sendInput = async (submitted: string, fromComposer: boolean) => {
    if (!canSubmitInput(submitted, submitState) || submitInFlight.current) return;
    submitInFlight.current = true;
    try {
      if (await workspace.send(submitted) && fromComposer) {
        setDraft(current => current === submitted ? '' : current);
      }
    } finally { submitInFlight.current = false; }
  };
  const requestInput = async (line: string, fromComposer: boolean) => {
    if (!canSubmitInput(line, submitState) || submitInFlight.current) return;
    const preset = permissionPresetFromCommand(line);
    if (preset && permissionNeedsConfirmation(preset, workspace.status)) {
      setPendingPermission({ preset, line, sessionId: workspace.sessionId, fromComposer });
      return;
    }
    await sendInput(line, fromComposer);
  };
  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    await requestInput(draft, true);
  };
  const confirmPermission = async () => {
    if (!pendingPermission || pendingPermission.sessionId !== workspace.sessionId) return;
    if (!canSubmitInput(pendingPermission.line, submitState)) return;
    const change = pendingPermission;
    setPendingPermission(null);
    await sendInput(change.line, change.fromComposer);
  };

  const empty = !workspace.loading && !workspace.projection.items.length && !workspace.outputs.length && !workspace.preview.text && !workspace.preview.tools.length;
  const editCount = workspace.projection.items.filter(item => item.type === 'tool' && ['edit', 'write'].includes(item.name)).length;
  const project = workspace.hello ? basename(workspace.hello.cwd) : workspace.hasProject ? 'Opening project' : 'No project open';
  const newTask = () => { setDraft(''); setSidebarOpen(false); void workspace.openSession(); };
  const openPanel = (tab: 'edits' | 'details') => {
    setDetailsOpen(current => panelTab === tab ? !current : true);
    setPanelTab(tab);
  };
  const composer = <div className="composer-region" hidden={!workspace.hasProject}>
    <form className={`composer ${running ? 'composer-running' : ''}`} onSubmit={event => void submit(event)}>
      {showCommands && <div className="command-suggestions" aria-label="Available commands">{commands.map(command => <button type="button" key={command.name} onClick={() => suggest(`/${command.name} `)}><code>/{command.name}</code><span>{command.summary}</span></button>)}</div>}
      <label htmlFor="composer" className="sr-only">Message Columbina</label>
      <textarea id="composer" ref={textarea} value={draft} disabled={unavailable} onChange={event => setDraft(event.target.value)} rows={2} placeholder={stopped ? 'Reconnect to continue this task' : running ? 'Add instructions to this task…' : 'Ask Columbina to work on your project…'} onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); void submit(); } }} />
      <div className="composer-toolbar">
        <div className="composer-options">
          <button type="button" className="model-button" onClick={() => setSettingsOpen(true)}><Sparkles size={13} /><span>{workspace.status?.model.model ?? 'Connecting'}</span><ChevronDown size={12} /></button>
          <button type="button" className="command-shortcut" onClick={() => suggest('/')} title="Commands"><Terminal size={14} /><span>Commands</span></button>
        </div>
        <div className="composer-send">{running && <button type="button" className="stop-button" onClick={() => void workspace.cancel()} disabled={workspace.cancelling} aria-label="Stop current turn" title="Stop current turn">{workspace.cancelling ? <LoaderCircle size={15} className="spin" /> : <Square size={12} fill="currentColor" />}</button>}<button type="submit" className="send-button" disabled={!canSubmit} aria-label={running ? 'Send follow-up' : 'Send message'}>{workspace.sending ? <LoaderCircle size={17} className="spin" /> : <ArrowUp size={18} />}</button></div>
      </div>
    </form>
    <div className="composer-caption"><span>{workspace.cancelling ? 'Stopping — waiting for the current turn to finish.' : running && draft.trim().startsWith('/') ? 'Commands are available when the agent is idle.' : running ? 'New instructions will be read at the next step.' : 'Enter to send · Shift + Enter for a new line'}</span><span><kbd>/</kbd> commands</span></div>
  </div>;

  return <div className={`window-frame ${window.columbina ? 'native-window' : 'browser-window'}`}>
    <header className="window-titlebar"><div className="window-app-name"><Moon size={15} /><span>Columbina</span></div><div className="window-project">{project}</div><MusicTitlebar music={music} />{!window.columbina && <span className="browser-preview-label">Desktop preview</span>}</header>
    {showIntro && <StartupVideo hasBackgroundMusic={music.hasTrack} muted={music.muted} onMutedChange={music.setMuted} onComplete={() => setShowIntro(false)} />}
    <div inert={showIntro} aria-hidden={showIntro || undefined} className={`app-shell ${sidebarCollapsed ? 'sidebar-collapsed' : ''} ${detailsOpen ? 'with-details' : ''}`} style={{ '--sidebar-width': `${sidebarWidth}px` } as React.CSSProperties}>
      <a href="#composer" className="skip-link">Skip to message input</a>
      {sidebarOpen && <button className="sidebar-scrim" aria-label="Close navigation" onClick={() => setSidebarOpen(false)} />}
      <aside className={`sidebar ${sidebarOpen ? 'is-open' : ''}`} aria-label="Project and tasks">
        <div className="sidebar-actions"><button className="new-session" disabled={workspace.loading || !workspace.connected || running} onClick={newTask}><SquarePen size={16} /><span>New task</span></button><button className="icon-button collapse-desktop" onClick={() => setSidebarCollapsed(true)} aria-label="Collapse sidebar" title="Collapse sidebar (Ctrl+B)"><PanelLeft size={16} /></button><button className="icon-button mobile-only" onClick={() => setSidebarOpen(false)} aria-label="Close navigation"><X size={17} /></button></div>
        <label className="session-search"><Search size={14} /><input value={search} onChange={event => setSearch(event.target.value)} placeholder="Search tasks" aria-label="Search tasks" /></label>
        <div className="sidebar-section-title"><span>Project</span><button className="icon-button" disabled={running || workspace.loading} onClick={() => void workspace.chooseFolder()} aria-label="Open project folder" title="Open project folder"><FolderOpen size={14} /></button></div>
        <div className="project-heading" title={workspace.hello?.cwd}><ChevronDown size={12} /><Folder size={15} /><strong>{project}</strong><span>{workspace.sessions.length}</span></div>
        <nav className="session-list" aria-label="Tasks">{workspace.sessions.filter(session => (session.first_input ?? session.id).toLowerCase().includes(search.toLowerCase())).map(session => <button key={session.id} className={`session-link ${workspace.sessionId === session.id ? 'active' : ''}`} disabled={workspace.loading || running} onClick={() => { setSidebarOpen(false); setDraft(''); void workspace.openSession(session.id); }} title={session.first_input ?? session.id}><span className={workspace.sessionId === session.id && running ? 'task-dot working' : 'task-dot'} /><span>{session.first_input ?? 'New task'}</span></button>)}
          {workspace.sessionId && !workspace.sessions.some(session => session.id === workspace.sessionId) && !search && <div className="session-link active"><span className="task-dot" /><span>New task</span></div>}
          {search && !workspace.sessions.some(session => (session.first_input ?? session.id).toLowerCase().includes(search.toLowerCase())) && <p className="empty-search">No tasks found.</p>}
        </nav>
        <div className="sidebar-bottom"><button className="open-project" disabled={running || workspace.loading} onClick={() => void workspace.chooseFolder()}><FolderOpen size={15} /><span>Open project</span></button><div className="sidebar-footer"><button onClick={() => setSettingsOpen(true)}><Settings2 size={15} /><span>Settings</span></button><button className="icon-button" onClick={() => { const index = themes.findIndex(item => item.id === theme); setTheme(themes[(index + 1) % themes.length].id); }} aria-label={`Switch theme, current ${theme}`} title="Switch theme">{theme === 'frostlight' ? <Sun size={15} /> : <Moon size={15} />}</button></div></div>
      </aside>
      {!sidebarCollapsed && <div className="sidebar-resizer" role="separator" aria-label="Sidebar width" aria-orientation="vertical" aria-valuenow={sidebarWidth} aria-valuemin={200} aria-valuemax={360} tabIndex={0} onKeyDown={event => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); setSidebarWidth(width => Math.min(360, Math.max(200, width + (event.key === 'ArrowLeft' ? -12 : 12)))); } }} onPointerDown={event => { event.currentTarget.setPointerCapture(event.pointerId); }} onPointerMove={event => { if (event.currentTarget.hasPointerCapture(event.pointerId)) setSidebarWidth(Math.min(360, Math.max(200, event.clientX))); }} onPointerUp={event => event.currentTarget.releasePointerCapture(event.pointerId)} />}
      <main className="main-workspace">
        <header className="topbar"><div className="task-heading"><button className="icon-button desktop-nav-toggle" onClick={() => setSidebarCollapsed(current => !current)} aria-label="Toggle sidebar" title="Toggle sidebar (Ctrl+B)"><PanelLeft size={16} /></button><button className="icon-button mobile-only" onClick={() => setSidebarOpen(true)} aria-label="Open navigation"><PanelLeft size={16} /></button><MessageSquare size={14} /><strong title={title}>{empty ? 'New task' : title}</strong></div><div className="topbar-actions"><button className="toolbar-button" aria-label="Toggle edit history" aria-pressed={detailsOpen && panelTab === 'edits'} onClick={() => openPanel('edits')}><FileDiff size={15} /><span>Edits</span>{editCount > 0 && <span className="count-badge">{editCount}</span>}</button><button className={`icon-button ${detailsOpen && panelTab === 'details' ? 'selected' : ''}`} aria-label="Toggle run details" title="Run details" onClick={() => openPanel('details')}><SlidersHorizontal size={16} /></button><button className="icon-button" title="Commands (Ctrl+K)" aria-label="Open commands" onClick={() => setPaletteOpen(true)}><Command size={16} /></button></div></header>
        {workspace.error && <div className="error-banner" role="alert"><Info size={15} /><span>{workspace.error}</span>{!workspace.connected && <button disabled={workspace.loading} onClick={() => void workspace.reconnect()}>Reconnect</button>}<button className="icon-button" aria-label="Dismiss error" onClick={workspace.clearError}><X size={15} /></button></div>}
        {stopped && <div className="error-banner" role="alert"><WifiOff size={16} /><span>{workspace.status?.detail || 'This agent has stopped.'}</span><button disabled={workspace.loading} onClick={() => void workspace.reconnect()}>Reconnect</button></div>}
        <div className={`task-body${empty || !workspace.hasProject ? ' task-body-empty' : ''}`}>
          <div className="task-conversation" hidden={empty || !workspace.hasProject}>
            <Conversation workspace={workspace} onLink={setLink} />
          </div>
          <div className="task-start" hidden={!empty && workspace.hasProject}>
            <div className="task-start-content">
              <span className="task-start-mark"><Moon size={30} /></span>
              <h1>{workspace.hasProject ? 'What are we working on?' : 'Open a project to begin'}</h1>
              {workspace.hasProject
                ? <div className="task-start-project"><Folder size={14} /><span>{project}</span></div>
                : <div className="open-project-start">
                  <p>Choose the folder Columbina will work in.</p>
                  <button ref={openProjectButton} className="primary-button" onClick={() => void workspace.chooseFolder()}>
                    <FolderOpen size={16} />Open project
                  </button>
                </div>}
            </div>
          </div>
          {composer}
          <div className="quick-actions" hidden={!empty || !workspace.hasProject}>
            <button onClick={() => suggest('Explore this codebase and explain how its main components fit together.')}><Search size={14} />Explore code</button>
            <button onClick={() => suggest('check.py fails. Find the mistake in calc.py, fix it, and verify the result.')}><Terminal size={14} />Fix a bug</button>
            <button onClick={() => suggest('Review the recent changes and explain the important differences.')}><FileDiff size={14} />Review changes</button>
          </div>
        </div>
        <footer className={`permission-strip ${dangerous ? 'elevated' : ''}`}><button onClick={() => setSettingsOpen(true)} title="View model and permissions">{dangerous ? <ShieldOff size={12} /> : <ShieldCheck size={12} />}<span>{workspace.status?.sandbox?.mode ?? 'No sandbox'}</span><span className="strip-divider">·</span><span>Approval: {workspace.status?.approval ?? 'unavailable'}</span><span className="strength">Isolation: {workspace.status?.sandbox?.strength ?? 'none'}</span></button><div className="status-right">{workspace.mode === 'mock' && <button className="replay-status" onClick={() => setSettingsOpen(true)} title="Recorded responses; no model requests or file operations">Replay mode</button>}<span className="run-status"><span className={running ? 'pulsing-dot' : 'small-dot'} />{workspace.cancelling ? 'Stopping' : running ? 'Working' : stopped ? 'Stopped' : workspace.connected ? 'Ready' : 'Offline'}</span><span className="token-count"><Gauge size={11} />{compact(workspace.projection.lastInputTokens)}</span></div></footer>
      </main>
      {detailsOpen && <aside className="inspector" aria-label="Task inspector"><div className="inspector-header"><div role="tablist" aria-label="Task information"><button role="tab" aria-selected={panelTab === 'edits'} onClick={() => setPanelTab('edits')}>Edits{editCount > 0 && <span>{editCount}</span>}</button><button role="tab" aria-selected={panelTab === 'details'} onClick={() => setPanelTab('details')}>Run details</button></div><button className="icon-button" onClick={() => setDetailsOpen(false)} aria-label="Close task inspector"><X size={15} /></button></div><div className="inspector-content" role="tabpanel">{panelTab === 'edits' ? <EditHistory items={workspace.projection.items} /> : <Details workspace={workspace} />}</div></aside>}
    {settingsOpen && <Dialog title="Settings" onClose={() => setSettingsOpen(false)} className="settings-dialog"><Appearance theme={theme} onTheme={setTheme} />
      <div className="intro-setting"><label><input type="checkbox" checked={playIntro} onChange={event => setPlayIntro(event.target.checked)} />Play opening video at startup</label><button className="secondary-button" disabled={running || workspace.loading || !!workspace.approval} onClick={() => { setSettingsOpen(false); setShowIntro(true); }}>Replay intro</button></div>
      <MusicSettings music={music} />
      <section className="settings-section">
        <div className="settings-section-heading"><h3>Model & permissions</h3><p>These settings apply to the active session.</p></div>
        <label className="field-label" htmlFor="model-setting">Model</label>
        <form className="inline-form" onSubmit={event => { event.preventDefault(); void requestInput(`/model ${modelDraft.trim()}`, false); }}>
          <input id="model-setting" value={modelDraft} onChange={event => setModelDraft(event.target.value)} disabled={!canModel || running || unavailable || workspace.sending} />
          <button className="secondary-button" disabled={!canModel || running || unavailable || workspace.sending || !modelDraft.trim()}>Apply</button>
        </form>
        <label className="field-label" htmlFor="permission-setting">Permission preset</label>
        <select id="permission-setting"
          value={workspace.status?.sandbox?.mode === 'danger-full-access' ? 'full-access' : workspace.status?.sandbox?.mode ?? ''}
          disabled={!canPermissions || running || unavailable || workspace.sending}
          onChange={event => void requestInput(`/permissions ${event.target.value}`, false)}>
          <option value="" disabled>Unavailable</option>
          <option value="read-only">Read only</option>
          <option value="workspace-write">Workspace write</option>
          <option value="full-access">Full access · commands run without approval</option>
        </select>
        {running && <p className="field-hint">Model and permission changes are available when the agent is idle.</p>}
      </section>
      <section className="settings-section connection-section"><div><h3>{workspace.mode === 'mock' ? 'Replay connection' : 'Python service'}</h3><p>{workspace.mode === 'mock' ? 'Try the full workflow using a recorded coding session. No model requests or file operations are made.' : 'Connected through the desktop app to the configured local service.'}</p><code>{workspace.hello?.cwd}</code></div><button className="secondary-button" disabled={running || workspace.loading} onClick={() => void workspace.reconnect()}>Reconnect</button></section>
      <div className="dialog-footer"><span>Columbina Desktop · Protocol {workspace.hello?.protocol ?? 1}</span><button className="primary-button" onClick={() => setSettingsOpen(false)}>Done</button></div>
    </Dialog>}
    {pendingPermission && <Dialog title={pendingPermission.preset === 'full-access' ? 'Allow full access?' : 'Expand workspace access?'} onClose={() => setPendingPermission(null)} className="permission-dialog">
      <p>{pendingPermission.preset === 'full-access'
        ? 'Columbina will run commands without sandbox restrictions or approval prompts.'
        : 'Columbina will be able to write files in this project. Operations requiring approval will still ask you.'}</p>
      <dl className="permission-change-summary">
        <div><dt>Project</dt><dd>{workspace.hello?.cwd ?? project}</dd></div>
        <div><dt>Current</dt><dd>{workspace.status?.sandbox?.mode ?? 'Unavailable'} · approval: {workspace.status?.approval ?? 'unavailable'}</dd></div>
        <div><dt>Requested</dt><dd>{pendingPermission.preset}</dd></div>
      </dl>
      <div className="dialog-actions">
        <button ref={permissionCancel} className="secondary-button" onClick={() => setPendingPermission(null)}>Cancel</button>
        <button className="primary-button" disabled={!canSubmitInput(pendingPermission.line, submitState)} onClick={() => void confirmPermission()}>
          {pendingPermission.preset === 'full-access' ? 'Enable full access' : 'Allow workspace writes'}
        </button>
      </div>
    </Dialog>}
    {paletteOpen && <Dialog title="Commands" onClose={() => setPaletteOpen(false)} className="palette-dialog"><p className="muted">Available in this workspace. Commands run when Columbina is idle.</p><div className="palette-list">{allCommands.map(command => <button key={command.name} onClick={() => { setPaletteOpen(false); suggest(`/${command.name} `); }}><Terminal size={16} /><span><strong>/{command.name}</strong><small>{command.summary}</small></span><ArrowDownToLine size={14} /></button>)}</div>{!allCommands.length && <p>Connect to a service to see its commands.</p>}</Dialog>}
    {link && <Dialog title="Open external link?" onClose={() => setLink(null)}><p className="muted">This address was included in the conversation.</p><pre className="external-url">{link}</pre><div className="dialog-actions"><button className="secondary-button" onClick={() => setLink(null)}>Cancel</button><button className="primary-button" disabled={!/^https?:\/\//i.test(link)} onClick={() => { if (window.columbina) void window.columbina.openExternal(link).then(() => setLink(null)).catch(workspace.fail); else { window.open(link, '_blank', 'noopener,noreferrer'); setLink(null); } }}>Open in browser <ArrowUpRight size={14} /></button></div></Dialog>}
    {workspace.approval && <Dialog title="Permission required" className="approval-dialog" onClose={() => { workspace.answer('rejected'); setAnswering(true); }}><div className="approval-intro"><span className="approval-icon"><ShieldCheck size={23} /></span><div><span className="eyebrow">{workspace.approval.tool}</span><p>{workspace.approval.summary}</p></div></div><label className="field-label">Full operation</label><pre className="approval-target">{workspace.approval.target ?? '(This tool did not specify a target)'}</pre><p className="approval-reason">{workspace.approval.reason}</p>{workspace.approval.standing && <p className="field-hint">Remembering approval applies only to {workspace.approval.standing}, for this service run.</p>}<div className="approval-actions"><button className="secondary-button" disabled={answering} onClick={() => { workspace.answer('rejected'); setAnswering(true); }}>Reject</button>{workspace.approval.standing && <button className="secondary-button" disabled={answering} onClick={() => { workspace.answer('allowed-once', true); setAnswering(true); }}>Allow & remember</button>}<button className="primary-button" disabled={answering} onClick={() => { workspace.answer('allowed-once'); setAnswering(true); }}>{answering ? 'Waiting…' : 'Allow once'}</button></div><div className="approval-bottom"><div className="approval-note"><CircleHelp size={13} />Waiting for your decision.</div><button className="secondary-button" disabled={workspace.cancelling} onClick={() => void workspace.cancel()}>{workspace.cancelling ? 'Stopping…' : 'Stop turn'}</button></div></Dialog>}
  </div></div>;
}
