import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ApprovalRequest, Hello, RpcMessage, SessionEvent, SessionStatus, SessionSummary, StreamNotification } from './protocol';
import { emptyPreview, projectEvents, reduceChunk } from './state';
import { createElectronTransport, RpcClient } from './transport';
import { createMockTransport } from './mock';

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
    active.current = null;
    buffered.current.clear();
    statuses.current.clear();
    approvalBuffer.current.clear();
    setLoading(true);
    setApproval(null);
    setPreview(emptyPreview());
    setOutputs([]);
    setError(null);
    setStatus(null);
    setCancelling(false);
    eventStore.current.clear();
    setEvents([]);
    try {
      const opened = await rpc.request<{ session_id: string; events: number }>('session/open', id ? { session_id: id } : {});
      if (version !== generation.current) return;
      active.current = opened.session_id;
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

  const connect = useCallback(async (cwd?: string, resumeId?: string) => {
    if (connecting.current || (window.columbina && !cwd)) return;
    connecting.current = true;
    connectionCleanup.current();
    client.current?.close();
    client.current = null;
    active.current = null;
    ++generation.current;
    setLoading(true);
    setConnected(false);
    setError(null);
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
        setError(reason || 'The service connection closed. Reconnect to continue.');
      });
      const offShutdown = bridge?.onShutdown(() => {
        void rpc.request('shutdown').catch(() => {}).finally(() => bridge.stop());
      });
      connectionCleanup.current = () => { unsubscribe(); offClose?.(); offShutdown?.(); };
      const greeting = await rpc.request<Hello>('initialize', { protocol: 1, client: { name: 'columbina-desktop', version: '0.1.0' } });
      setHello(greeting);
      setConnected(true);
      await openSession(resumeId);
    } catch (problem) {
      fail(problem);
      setLoading(false);
    } finally { connecting.current = false; }
  }, [fail, openSession, publish, refreshSessions]);

  useEffect(() => {
    if (!window.columbina) void connect();
    return () => { ++generation.current; connectionCleanup.current(); client.current?.close(); };
  }, [connect]);

  const command = useCallback(async (line: string) => {
    if (!client.current) return;
    const result = await client.current.request<{ text: string }>('session/command', { line });
    if (result.text) setOutputs(current => [...current, { id: Date.now(), line, text: result.text }]);
  }, []);

  const send = useCallback(async (text: string) => {
    if (!client.current || !text.trim()) return false;
    setSending(true);
    setError(null);
    try {
      if (text.trim().startsWith('/')) await command(text.trim());
      else await client.current.request(status?.status === 'running' ? 'session/steer' : 'session/prompt', { text });
      return true;
    } catch (problem) { fail(problem); return false; }
    finally { setSending(false); }
  }, [command, fail, status]);

  const cancel = useCallback(async () => {
    if (!client.current) return;
    setCancelling(true);
    try { await client.current.request('session/cancel'); }
    catch (problem) { fail(problem); setCancelling(false); }
  }, [fail]);

  const answer = useCallback((outcome: 'allowed-once' | 'rejected', remember = false) => {
    if (!approval || !client.current) return;
    client.current.respond(approval.requestId, { outcome, remember });
  }, [approval]);

  const chooseFolder = useCallback(async () => {
    if (!window.columbina) { setError('Folder selection is available in the desktop app. This preview uses the replay workspace.'); return; }
    if (choosingDirectory.current || connecting.current) return;
    choosingDirectory.current = true;
    try {
      const folder = await window.columbina.selectFolder();
      if (folder) {
        selectedDirectory.current = folder;
        setHasProject(true);
        await connect(folder);
      }
    } catch (problem) { fail(problem); }
    finally { choosingDirectory.current = false; }
  }, [connect, fail]);

  const projection = useMemo(() => projectEvents(events), [events]);
  return { hello, sessionId, sessions, events, status, preview, approval, outputs, connected, loading, hasProject, sending, cancelling, error, mode, projection, openSession, send, cancel, answer, command, chooseFolder, reconnect: () => connect(selectedDirectory.current ?? undefined, sessionId ?? undefined), clearError: () => setError(null), fail };
}

export type Workspace = ReturnType<typeof useWorkspace>;
