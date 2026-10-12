import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ApprovalRequest, Hello, RpcMessage, SessionEvent, SessionStatus, SessionSummary, StreamNotification } from './protocol';
import { emptyPreview, projectEvents, reduceChunk } from './state';
import { createElectronTransport, RpcClient } from './transport';
import { listedOf, type Listed } from './key/form';
import { createMockTransport } from './mock';
import { isCommand } from './uiActions';

export interface PendingApproval extends ApprovalRequest { requestId: string | number }
export interface CommandOutput { id: number; line: string; text: string }

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
  // The folders that were chosen at earlier times, as the shell keeps them.
  const [recent, setRecent] = useState<{ path: string; name: string }[]>([]);
  // The services of the model and where the key of each comes from, as the program of the
  // service tells of them. Null where the application does not run that program, or before it answered.
  const [services, setServices] = useState<Listed | null>(null);
  // The last lines that the service wrote for errors, asked for when its connection ends.
  const [lastWords, setLastWords] = useState<string[]>([]);
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
  const connecting = useRef(false);
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

  const openSession = useCallback(async (id?: string) => {
    const rpc = client.current;
    if (!rpc || opening.current) return;
    opening.current = true;
    const version = ++generation.current;
    // What comes for the other task before it is the one shown is kept until then.
    buffered.current.clear();
    approvalBuffer.current.clear();
    setLoading(true);
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
      setSessionId(opened.session_id);
      setStatus(statuses.current.get(opened.session_id) ?? null);
      setApproval(approvalBuffer.current.get(opened.session_id) ?? null);
      publish(buffered.current.get(opened.session_id) ?? []);
      let from = 0;
      while (from < opened.events) {
        const page = await rpc.request<{ events: SessionEvent[]; next: number | null }>('session/events', { from, limit: Math.min(500, opened.events - from) });
        if (version !== generation.current) return;
        publish(page.events.filter(event => event.seq < opened.events));
        const next = page.next ?? opened.events;
        if (next <= from) throw new Error('The service did not advance the history page. Reconnect to try again.');
        from = next;
      }
      await refreshSessions();
    } catch (problem) {
      if (version === generation.current) fail(problem);
    } finally {
      opening.current = false;
      if (version === generation.current) setLoading(false);
    }
  }, [fail, publish, refreshSessions]);

  const refreshRecent = useCallback(async () => {
    try {
      setRecent(await window.columbina?.recentProjects() ?? []);
    } catch {
      // Without the list, a folder can still be chosen in the dialog.
    }
  }, []);

  const refreshServices = useCallback(async () => {
    try {
      setServices(listedOf(await window.columbina?.services()));
    } catch (problem) {
      setServices(null);
      fail(problem);
    }
  }, [fail]);

  const connect = useCallback(async (cwd?: string, resumeId?: string) => {
    if (connecting.current || (window.columbina && !cwd)) return;
    connecting.current = true;
    if (cwd) setFolder(cwd);
    connectionCleanup.current();
    client.current?.close();
    client.current = null;
    active.current = null;
    ++generation.current;
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
        const started = await bridge.start({ cwd: cwd! });
        selectedDirectory.current = started.cwd;
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
        setClosed(reason || '');
        // Why it ended is most often in what it wrote last.
        void bridge?.serviceTail().then(setLastWords).catch(() => {});
      });
      const offShutdown = bridge?.onShutdown(() => {
        void rpc.request('shutdown').catch(() => {}).finally(() => bridge.stop());
      });
      connectionCleanup.current = () => { unsubscribe(); offClose?.(); offShutdown?.(); };
      const greeting = await rpc.request<Hello>('initialize', { protocol: 1, client: { name: 'columbina-desktop', version: '0.1.0' } });
      setHello(greeting);
      setConnected(true);
      void refreshRecent();
      void refreshServices();
      await openSession(resumeId);
    } catch (problem) {
      fail(problem);
      setLoading(false);
    } finally { connecting.current = false; }
  }, [fail, openSession, publish, refreshRecent, refreshServices, refreshSessions]);

  useEffect(() => {
    if (!window.columbina) void connect();
    void refreshRecent();
    return () => { ++generation.current; connectionCleanup.current(); client.current?.close(); };
  }, [connect, refreshRecent]);

  const command = useCallback(async (line: string) => {
    if (!client.current) return;
    const result = await client.current.request<{ text: string }>('session/command', { line });
    if (result.text) setOutputs(current => [...current, { id: Date.now(), line, text: result.text }]);
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
   * works. Whatever the text looks like, it is never carried out as a command.
   */
  const prompt = useCallback(async (text: string) => {
    const rpc = client.current;
    if (!rpc || !text.trim()) return false;
    return sent(() => rpc.request(status?.status === 'running' ? 'session/steer' : 'session/prompt', { text }));
  }, [sent, status]);

  /** Send what was written in the field: a command if it begins with a slash, and input for the agent otherwise. */
  const send = useCallback(async (text: string) => {
    if (!isCommand(text)) return prompt(text);
    if (!client.current) return false;
    return sent(() => command(text.trim()));
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
   * Open the window in which a service is chosen and its key is given.
   *
   * Returns:
   *   Whether something was stored there. The key itself never comes to this window.
   */
  const setUpService = useCallback(async (service: string | null, language: string): Promise<boolean> => {
    if (!window.columbina) return false;
    try {
      const { stored } = await window.columbina.setUpService({ service, language });
      if (stored) await refreshServices();
      return stored;
    } catch (problem) {
      fail(problem);
      return false;
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
    selectedDirectory.current = chosen;
    setHasProject(true);
    await connect(chosen);
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

  const projection = useMemo(() => projectEvents(events), [events]);
  return {
    hello, sessionId, sessions, events, status, preview, approval, outputs, connected, loading, hasProject, sending,
    cancelling, error, closed, lastWords, openedAt, folder, recent, services, mode, projection,
    openSession, send, prompt, cancel, answer, command, pickFolder, pickRecent, openFolder, setUpService,
    reconnect: () => connect(selectedDirectory.current ?? undefined, sessionId ?? undefined),
    showLog: () => void window.columbina?.showLog().catch(fail),
    clearError: () => setError(null), fail,
  };
}

export type Workspace = ReturnType<typeof useWorkspace>;
