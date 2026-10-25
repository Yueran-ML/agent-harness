import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { skillsOf } from './app/tell';
import type {
  ApprovalRequest, Hello, RpcMessage, SessionEvent, SessionStatus, SessionSummary, SkillSummary, StreamNotification,
} from './protocol';
import { emptyPreview, projectEvents, reduceChunk } from './state';
import { createElectronTransport, RpcClient } from './transport';
import { HISTORY_PAGE } from './app/tell';
import { listedOf, type Listed } from './key/form';
import { createMockTransport } from './mock';
import { isCommand } from './uiActions';

export interface PendingApproval extends ApprovalRequest { requestId: string | number }
/**
 * What a command answered. The log does not hold it, so it is gone when the task is opened
 * again. `after` is the number of the last event that the log had when the answer came, and
 * -1 where it had none: that is where the answer stands in the conversation.
 */
export interface CommandOutput { id: number; line: string; text: string; after: number }

export function useWorkspace() {
  const [hello, setHello] = useState<Hello | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [status, setStatus] = useState<SessionStatus | null>(null);
  const [preview, setPreview] = useState(emptyPreview);
  const [approval, setApproval] = useState<PendingApproval | null>(null);
  const [outputs, setOutputs] = useState<CommandOutput[]>([]);
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(!window.columbina);
  const [hasProject, setHasProject] = useState(!window.columbina);
  const [sending, setSending] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Why the connection to the service ended, as the shell said it. Null while it holds.
  const [closed, setClosed] = useState<string | null>(null);
  // How many events the log of the task held when it was opened. What came before is history.
  const [openedAt, setOpenedAt] = useState(0);
  // The folder of the project, from the moment it is chosen.
  const [folder, setFolder] = useState<string | null>(null);
  // Whether what is open is a conversation that belongs to no project, in a folder that the shell names.
  const [chat, setChat] = useState(false);
  const inChat = useRef(false);
  // The folders that were chosen at earlier times, as the shell keeps them.
  const [recent, setRecent] = useState<{ path: string; name: string }[]>([]);
  // The services of the model and where the key of each comes from, as the program of the
  // service tells of them. Null where the application does not run that program, or before it answered.
  const [services, setServices] = useState<Listed | null>(null);
  // Whether the program is being asked for the services. It is asked as soon as the page is
  // there, also before any project is open: the first page tells whether a service is set up.
  const [checkingServices, setCheckingServices] = useState(false);
  // The asks for the services, by number. Only the answer to the last one counts.
  const servicesAsked = useRef(0);
  // The skills of the task that is open, as the service process lists them.
  const [skills, setSkills] = useState<SkillSummary[]>([]);
  // The last lines that the service wrote for errors, asked for when its connection ends.
  const [lastWords, setLastWords] = useState<string[]>([]);
  // How far the log of a long task is read. Null for a log of one page, and when none is read.
  const [reading, setReading] = useState<{ read: number; of: number } | null>(null);
  const [mode, setMode] = useState<'mock' | 'service'>('mock');
  const client = useRef<RpcClient | null>(null);
  const active = useRef<string | null>(null);
  const generation = useRef(0);
  const eventStore = useRef(new Map<number, SessionEvent>());
  const buffered = useRef(new Map<string, SessionEvent[]>());
  const statuses = useRef(new Map<string, SessionStatus>());
  const approvalBuffer = useRef(new Map<string, PendingApproval>());
  const connectionCleanup = useRef<() => void>(() => {});
  const streamKey = useRef('');
  const opening = useRef(false);
  // Whether the task that is being opened has answered, and its log is being read.
  const paging = useRef(false);
  const connecting = useRef(false);
  // The starts that were asked for, by number. One that is given up is no longer the last.
  const attempts = useRef(0);
  const choosingDirectory = useRef(false);
  const selectedDirectory = useRef<string | null>(null);

  const fail = useCallback((problem: unknown) => setError(problem instanceof Error ? problem.message : String(problem)), []);
  const publish = useCallback((batch: SessionEvent[]) => {
    for (const event of batch) eventStore.current.set(event.seq, event);
    setEvents([...eventStore.current.values()].sort((a, b) => a.seq - b.seq));
  }, []);

  const refreshSessions = useCallback(async () => {
    if (!client.current) return;
    const result = await client.current.request<{ sessions: SessionSummary[] }>('session/list');
    setSessions(result.sessions);
  }, []);

  const refreshSkills = useCallback(async () => {
    const rpc = client.current;
    const asked = active.current;
    if (!rpc || asked === null) return;
    let found: SkillSummary[] = [];
    try {
      found = skillsOf(await rpc.request('session/skills'));
    } catch {
      // A service process of an earlier day does not know the request. There are no skills to name then.
    }
    // The answer is of the task that was open when it was asked for.
    if (active.current === asked) setSkills(found);
  }, []);

  const openSession = useCallback(async (id?: string) => {
    const rpc = client.current;
    // While the log of a task is being read, another task may be opened in its place.
    if (!rpc || (opening.current && !paging.current)) return;
    opening.current = true;
    paging.current = false;
    const version = ++generation.current;
    // What comes for the other task before it is the one shown is kept until then.
    buffered.current.clear();
    approvalBuffer.current.clear();
    setLoading(true);
    setReading(null);
    setError(null);
    try {
      const opened = await rpc.request<{ session_id: string; events: number }>('session/open', id ? { session_id: id } : {});
      if (version !== generation.current) return;
      // Only now is the task that was shown left behind. Had the other one not opened, this one
      // would still be here, with whatever runs in it: the service keeps it then.
      active.current = opened.session_id;
      eventStore.current.clear();
      setEvents([]);
      setPreview(emptyPreview());
      streamKey.current = '';
      setOutputs([]);
      setCancelling(false);
      setOpenedAt(opened.events);
      setSkills([]);
      setSessionId(opened.session_id);
      setStatus(statuses.current.get(opened.session_id) ?? null);
      setApproval(approvalBuffer.current.get(opened.session_id) ?? null);
      publish(buffered.current.get(opened.session_id) ?? []);
      paging.current = true;
      // A log of more than one page takes long enough to be told of.
      const long = opened.events > HISTORY_PAGE;
      if (long) setReading({ read: 0, of: opened.events });
      let from = 0;
      while (from < opened.events) {
        const page = await rpc.request<{ events: SessionEvent[]; next: number | null }>('session/events', { from, limit: Math.min(HISTORY_PAGE, opened.events - from) });
        if (version !== generation.current) return;
        publish(page.events.filter(event => event.seq < opened.events));
        const next = page.next ?? opened.events;
        if (next <= from) throw new Error('The service did not advance the history page. Reconnect to try again.');
        from = next;
        if (long) setReading({ read: Math.min(from, opened.events), of: opened.events });
      }
      await refreshSessions();
      await refreshSkills();
    } catch (problem) {
      if (version === generation.current) fail(problem);
    } finally {
      // Only the opening that is still wanted puts things back. One that another overtook is
      // over without a word, and the other has the say.
      if (version === generation.current) {
        opening.current = false;
        paging.current = false;
        setLoading(false);
        setReading(null);
      }
    }
  }, [fail, publish, refreshSessions, refreshSkills]);

  const refreshRecent = useCallback(async () => {
    try {
      setRecent(await window.columbina?.recentProjects() ?? []);
    } catch {
      // Without the list, a folder can still be chosen in the dialog.
    }
  }, []);

  const refreshServices = useCallback(async (): Promise<Listed | null> => {
    const number = ++servicesAsked.current;
    setCheckingServices(true);
    try {
      const listed = listedOf(await window.columbina?.services());
      if (number === servicesAsked.current) setServices(listed);
      return listed;
    } catch (problem) {
      if (number === servicesAsked.current) {
        setServices(null);
        fail(problem);
      }
      return null;
    } finally {
      if (number === servicesAsked.current) setCheckingServices(false);
    }
  }, [fail]);
  const connect = useCallback(async (cwd?: string, resumeId?: string) => {
    if (connecting.current || (window.columbina && !cwd && !inChat.current)) return;
    connecting.current = true;
    const attempt = ++attempts.current;
    // A start that was given up goes on to its next wait, and is to change nothing after that.
    const wanted = () => attempt === attempts.current;
    if (cwd) setFolder(cwd);
    connectionCleanup.current();
    client.current?.close();
    client.current = null;
    active.current = null;
    ++generation.current;
    // Whatever task was being opened belonged to the service before this one.
    opening.current = false;
    paging.current = false;
    setReading(null);
    setLoading(true);
    setConnected(false);
    setError(null);
    setClosed(null);
    setLastWords([]);
    setHello(null);
    setStatus(null);
    setSessionId(null);
    setSessions([]);
    eventStore.current.clear();
    setEvents([]);
    setOutputs([]);
    setPreview(emptyPreview());
    setApproval(null);
    try {
      const bridge = window.columbina;
      if (bridge) {
        await bridge.stop();
        if (!wanted()) return;
        const started = inChat.current ? await bridge.startChat() : await bridge.start({ cwd: cwd! });
        if (!wanted()) return;
        // A conversation in no project is started again as one, wherever its folder is.
        if (!inChat.current) selectedDirectory.current = started.cwd;
        setMode(started.mode);
      } else setMode('mock');
      const transport = bridge ? createElectronTransport(bridge) : createMockTransport({ cwd });
      const rpc = new RpcClient(transport);
      client.current = rpc;
      const unsubscribe = rpc.subscribe((message: RpcMessage) => {
        if (!('method' in message)) return;
        const params = message.params ?? {};
        const id = String(params.session_id ?? '');
        if (message.method === 'session.status') {
          const next = params as unknown as SessionStatus;
          statuses.current.set(id, next);
          if (active.current === id) {
            setStatus(next);
            if (next.status !== 'running') setCancelling(false);
          }
        } else if (message.method === 'session.event') {
          const event = params.event as SessionEvent;
          if (!event || typeof event.seq !== 'number') return;
          if (active.current === id) {
            publish([event]);
            if (event.type === 'assistant/message' || event.type === 'assistant/attempt' || event.type === 'turn/end') {
              setPreview(emptyPreview());
              streamKey.current = '';
            }
            if (event.type === 'turn/end') void refreshSessions().catch(fail);
          } else {
            const batch = buffered.current.get(id) ?? [];
            batch.push(event);
            buffered.current.set(id, batch);
          }
          if (event.type === 'approval/decided') {
            const call = event.data.call_id;
            if (approvalBuffer.current.get(id)?.call_id === call) approvalBuffer.current.delete(id);
            setApproval(current => current?.session_id === id && current.call_id === call ? null : current);
          }
        } else if (message.method === 'session.stream' && active.current === id) {
          const stream = params as unknown as StreamNotification;
          const key = `${id}:${stream.turn}:${stream.step}`;
          const reset = streamKey.current !== key;
          streamKey.current = key;
          setPreview(current => reduceChunk(reset ? emptyPreview() : current, stream.chunk));
        } else if (message.method === 'approval/ask' && 'id' in message) {
          const pending = { ...params, requestId: message.id } as unknown as PendingApproval;
          approvalBuffer.current.set(id, pending);
          if (active.current === id) setApproval(pending);
        }
      });
      const offClose = transport.onClose?.(reason => {
        setConnected(false);
        setLoading(false);
        setApproval(null);
        setPreview(emptyPreview());
        // Whatever the service was doing went with it. Its last word was that a turn runs.
        setStatus(current => (current?.status === 'running' ? { ...current, status: 'idle' } : current));
        setCancelling(false);
        setClosed(reason || '');
        // Why it ended is most often in what it wrote last.
        void bridge?.serviceTail().then(setLastWords).catch(() => {});
      });
      const offShutdown = bridge?.onShutdown(() => {
        void rpc.request('shutdown').catch(() => {}).finally(() => bridge.stop());
      });
      connectionCleanup.current = () => { unsubscribe(); offClose?.(); offShutdown?.(); };
      const greeting = await rpc.request<Hello>('initialize', { protocol: 1, client: { name: 'columbina-desktop', version: '0.1.0' } });
      if (!wanted()) return;
      setHello(greeting);
      setConnected(true);
      void refreshRecent();
      void refreshServices();
      await openSession(resumeId);
    } catch (problem) {
      if (wanted()) {
        fail(problem);
        setLoading(false);
      }
    } finally {
      // A start that was given up is put away by whoever gave it up.
      if (wanted()) connecting.current = false;
    }
  }, [fail, openSession, publish, refreshRecent, refreshServices, refreshSessions]);

  /**
   * Give up a start that has not been answered, and go back to where no project is open.
   *
   * The service may be starting, or may have started and say nothing. Either way it is
   * told to stop. The shell takes its orders one after the other, so this one comes after
   * the start that is under way and before any start that is asked for later: no service is
   * left behind, and none that is started next is ended by it.
   */
  const cancelStart = useCallback(async () => {
    if (!connecting.current) return;
    ++attempts.current;
    ++generation.current;
    connectionCleanup.current();
    connectionCleanup.current = () => {};
    client.current?.close();
    client.current = null;
    active.current = null;
    opening.current = false;
    paging.current = false;
    selectedDirectory.current = null;
    inChat.current = false;
    setChat(false);
    setLoading(false);
    setConnected(false);
    setReading(null);
    setHello(null);
    setStatus(null);
    setClosed(null);
    setError(null);
    setFolder(null);
    setHasProject(false);
    connecting.current = false;
    try {
      await window.columbina?.stop();
    } catch (problem) {
      fail(problem);
    }
  }, [fail]);

  useEffect(() => {
    if (!window.columbina) void connect();
    // The first page tells of the projects of earlier days, and of whether a service is set up.
    void refreshRecent();
    if (window.columbina) void refreshServices();
    return () => { ++generation.current; connectionCleanup.current(); client.current?.close(); };
  }, [connect, refreshRecent, refreshServices]);

  /**
   * Have a command carried out.
   *
   * Args:
   *   line: The command.
   *   quiet: Whether what it answers is left out of the conversation. A change that the user
   *     made in the settings shows there, and a line of the service about it would stand in
   *     a conversation in which nobody typed a command.
   */
  const command = useCallback(async (line: string, quiet = false) => {
    if (!client.current) return;
    const result = await client.current.request<{ text: string }>('session/command', { line });
    if (!result.text || quiet) return;
    // What the command did is in the log by now: the service tells of it before it answers.
    let after = -1;
    for (const seq of eventStore.current.keys()) if (seq > after) after = seq;
    setOutputs(current => [...current, { id: (current.at(-1)?.id ?? 0) + 1, line, text: result.text, after }]);
  }, []);

  const sent = useCallback(async (give: () => Promise<unknown>) => {
    setSending(true);
    setError(null);
    try {
      await give();
      return true;
    } catch (problem) {
      fail(problem);
      return false;
    } finally {
      setSending(false);
    }
  }, [fail]);

  /**
   * Give the agent something to read: a task while it rests, words for its next step while it
   * works. With `now` the step that runs is cut short, so that the agent reads them at once.
   * Whatever the text looks like, it is never carried out as a command.
   */
  const prompt = useCallback(async (text: string, now = false) => {
    const rpc = client.current;
    if (!rpc || !text.trim()) return false;
    if (status?.status !== 'running') return sent(() => rpc.request('session/prompt', { text }));
    return sent(() => rpc.request('session/steer', now ? { text, now } : { text }));
  }, [sent, status]);

  /**
   * Send a line: a command if it begins with a slash, and input for the agent otherwise.
   * `quiet` keeps what a command answers out of the conversation, see `command`. `now` has a
   * turn that runs read the input at once.
   */
  const send = useCallback(async (text: string, quiet = false, now = false) => {
    if (!isCommand(text)) return prompt(text, now);
    if (!client.current) return false;
    return sent(() => command(text.trim(), quiet));
  }, [command, prompt, sent]);

  const cancel = useCallback(async () => {
    if (!client.current) return;
    setCancelling(true);
    try { await client.current.request('session/cancel'); }
    catch (problem) { fail(problem); setCancelling(false); }
  }, [fail]);

  /**
   * Answer a question of the agent.
   *
   * Args:
   *   requestId: The question that was on the screen when the answer was given. An answer
   *     is for that question and for no other: if another one has taken its place, nothing is sent.
   */
  const answer = useCallback((requestId: string | number, outcome: 'allowed-once' | 'rejected', remember = false) => {
    if (!approval || approval.requestId !== requestId || !client.current) return;
    client.current.respond(requestId, { outcome, remember });
  }, [approval]);

  /**
   * Open the window in which a service is chosen and its key is given, and wait until it is
   * closed. The answer is the services as they are set up afterwards, or null where nothing
   * was stored or deleted there. The key itself never comes to this window.
   */
  const setUpService = useCallback(async (service: string | null, language: string): Promise<{ listed: Listed | null } | null> => {
    if (!window.columbina) return null;
    try {
      const { changed } = await window.columbina.setUpService({ service, language });
      return changed ? { listed: await refreshServices() } : null;
    } catch (problem) {
      fail(problem);
      return null;
    }
  }, [fail, refreshServices]);

  /**
   * Let the user choose a folder in the dialog of the system. Null if none was chosen.
   *
   * Args:
   *   language: The language of the words that are shown, as a page names it. The dialog is
   *     titled in it.
   */
  const pickFolder = useCallback(async (language: string): Promise<string | null> => {
    if (!window.columbina) { setError('Folder selection is available in the desktop app. This preview uses the replay workspace.'); return null; }
    if (choosingDirectory.current || connecting.current) return null;
    choosingDirectory.current = true;
    try {
      return await window.columbina.selectFolder({ language });
    } catch (problem) {
      fail(problem);
      return null;
    } finally {
      choosingDirectory.current = false;
    }
  }, [fail]);

  /** Start the service in a folder that the shell has let through, and open a task there. */
  const openFolder = useCallback(async (chosen: string) => {
    inChat.current = false;
    setChat(false);
    selectedDirectory.current = chosen;
    setHasProject(true);
    await connect(chosen);
  }, [connect]);

  /** Start a conversation that belongs to no project, and open a task in it. */
  const openChat = useCallback(async () => {
    if (connecting.current) return;
    inChat.current = true;
    setChat(true);
    selectedDirectory.current = null;
    setFolder(null);
    setHasProject(true);
    await connect();
  }, [connect]);

  /** The folder to start in for one of the recent projects, or null if the shell does not let it through. */
  const pickRecent = useCallback(async (kept: string): Promise<string | null> => {
    if (!window.columbina || connecting.current) return null;
    try {
      return await window.columbina.openRecent(kept);
    } catch (problem) {
      fail(problem);
      // It may be gone, or lead somewhere else now. The list says what is still there.
      void refreshRecent();
      return null;
    }
  }, [fail, refreshRecent]);

  const projection = useMemo(() => projectEvents(events, closed !== null), [events, closed]);
  return {
    hello, sessionId, sessions, events, status, preview, approval, outputs, connected, loading, hasProject, sending,
    cancelling, error, closed, lastWords, openedAt, folder, chat, recent, services, checkingServices, skills, mode, projection,
    reading, openSession, send, prompt, cancel, answer, command, pickFolder, pickRecent, openFolder, openChat, setUpService,
    refreshServices, cancelStart,
    reconnect: () => connect(selectedDirectory.current ?? undefined, sessionId ?? undefined),
    showLog: () => void window.columbina?.showLog().catch(fail),
    clearError: () => setError(null), fail,
  };
}

export type Workspace = ReturnType<typeof useWorkspace>;
