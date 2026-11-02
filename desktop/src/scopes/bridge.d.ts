import type { EngineAnswer, EngineEvent } from '../studio/engine';

/** What the floating window of the video scopes is told of what it measures. */
export interface ScopesWindowContext {
  /** The engine open in the studio, or none. */
  session: number | null;
  /** Whether the timeline viewer has something loaded: until then a scope has nothing to measure. */
  loaded: boolean;
  language: string;
  theme: string;
}

/** What the floating window of the video scopes can ask of the shell. It can ask nothing else of the system. */
export interface ScopesBridge {
  context(): Promise<ScopesWindowContext>;
  onContext(listener: (context: ScopesWindowContext) => void): () => void;
  /** `scopes.subscribe`, `scopes.set` and `scopes.unsubscribe` of its own scopes. */
  call(method: string, params?: Record<string, unknown>): Promise<EngineAnswer>;
  /** The pictures of its scopes, as `scopes.frame` notifications of the engine. */
  onEvent(listener: (event: EngineEvent) => void): () => void;
  close(): Promise<void>;
}

declare global {
  interface Window { columbinaScopes?: ScopesBridge }
}
