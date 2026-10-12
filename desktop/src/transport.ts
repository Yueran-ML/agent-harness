import type { RpcId, RpcMessage } from './protocol';

export interface Transport {
  send(message: RpcMessage): void;
  subscribe(listener: (message: RpcMessage) => void): () => void;
  close(): void;
  onClose?(listener: (reason: string) => void): () => void;
}

export class RpcError extends Error {
  constructor(public readonly code: number, message: string, public readonly data?: unknown) {
    super(message);
    this.name = 'RpcError';
  }
}

/** Both peers own their id namespace; a server request must never resolve a client request. */
export class RpcClient {
  private nextId = 1;
  private pending = new Map<RpcId, { resolve: (result: unknown) => void; reject: (error: Error) => void }>();
  private listeners = new Set<(message: RpcMessage) => void>();
  private closeListeners = new Set<(reason: string) => void>();
  private closed = false;
  private unsubscribe: () => void;
  private unsubscribeClose?: () => void;

  constructor(private readonly transport: Transport) {
    this.unsubscribe = transport.subscribe((message) => this.receive(message));
    this.unsubscribeClose = transport.onClose?.((reason) => this.disconnected(reason));
  }

  request<T = unknown>(method: string, params?: Record<string, unknown>): Promise<T> {
    if (this.closed) return Promise.reject(new Error('The service connection is closed.'));
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: (value) => resolve(value as T), reject });
      try {
        this.transport.send({ jsonrpc: '2.0', id, method, ...(params === undefined ? {} : { params }) });
      } catch (error) {
        this.pending.delete(id);
        reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  subscribe(listener: (message: RpcMessage) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  onClose(listener: (reason: string) => void): () => void {
    this.closeListeners.add(listener);
    return () => this.closeListeners.delete(listener);
  }

  respond(id: RpcId, result: unknown): void {
    if (!this.closed) this.transport.send({ jsonrpc: '2.0', id, result });
  }

  private receive(message: RpcMessage): void {
    if (this.closed) return;
    if ('method' in message) {
      for (const listener of this.listeners) listener(message);
      return;
    }
    if (message.id === null) return;
    const pending = this.pending.get(message.id);
    if (!pending) return;
    this.pending.delete(message.id);
    if ('error' in message) pending.reject(new RpcError(message.error.code, message.error.message, message.error.data));
    else pending.resolve(message.result);
  }

  private disconnected(reason: string): void {
    if (this.closed) return;
    this.closed = true;
    for (const pending of this.pending.values()) pending.reject(new Error(reason));
    this.pending.clear();
    for (const listener of this.closeListeners) listener(reason);
    this.listeners.clear();
  }

  close(): void {
    this.disconnected('The service connection is closed.');
    this.unsubscribe();
    this.unsubscribeClose?.();
    this.transport.close();
  }
}

export function createElectronTransport(bridge = window.columbina): Transport {
  if (!bridge) throw new Error('The Electron service bridge is not available on this page.');
  const listeners = new Set<(message: RpcMessage) => void>();
  const closeListeners = new Set<(reason: string) => void>();
  let closed = false;
  const ended = (reason: string) => {
    if (closed) return;
    closed = true;
    for (const listener of closeListeners) listener(reason);
  };
  const removeLine = bridge.onLine((line) => {
    if (closed) return;
    let message: RpcMessage;
    try {
      message = JSON.parse(line) as RpcMessage;
      if (!message || message.jsonrpc !== '2.0') throw new Error('Invalid JSON-RPC message');
    } catch {
      ended('The service sent an invalid protocol message.');
      void bridge.stop();
      return;
    }
    for (const listener of listeners) listener(message);
  });
  const removeClose = bridge.onClose(ended);
  return {
    send(message) {
      if (closed) throw new Error('The service connection is closed.');
      bridge.send(JSON.stringify(message));
    },
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    onClose(listener) { closeListeners.add(listener); return () => closeListeners.delete(listener); },
    close() {
      ended('The service connection is closed.');
      removeLine(); removeClose(); listeners.clear(); closeListeners.clear();
      void bridge.stop();
    },
  };
}
