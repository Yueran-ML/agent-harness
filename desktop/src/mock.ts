import { createReplayService } from './replay.shared.js';
import type { RpcMessage } from './protocol';
import type { Transport } from './transport';

export function createMockTransport(options: { cwd?: string; delayMs?: number } = {}): Transport {
  const listeners = new Set<(message: RpcMessage) => void>();
  const closeListeners = new Set<(reason: string) => void>();
  let closed = false;
  const service = createReplayService((message: RpcMessage) => {
    queueMicrotask(() => { if (!closed) for (const listener of listeners) listener(message); });
  }, options);
  return {
    send(message) {
      if (closed) throw new Error('The replay service is closed.');
      void service.receive(message);
    },
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    onClose(listener) { closeListeners.add(listener); return () => closeListeners.delete(listener); },
    close() {
      if (closed) return;
      service.close(); closed = true;
      for (const listener of closeListeners) listener('The replay service is closed.');
      listeners.clear(); closeListeners.clear();
    },
  };
}
