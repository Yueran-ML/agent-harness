import { useCallback, useRef, useState } from 'react';
import { MODELS_FRESH_MS } from './app/models';
import { checkedOf, type Failure } from './key/form';

/** What is known of the models of one service. */
export type Models =
  /** The service is being asked. What it listed before is shown meanwhile, where it listed any. */
  | { kind: 'asking'; models: string[] }
  | { kind: 'listed'; models: string[]; at: number }
  | { kind: 'failed'; failure: Failure; said: string; at: number };

const said = (problem: unknown) => (problem instanceof Error ? problem.message : String(problem));

/**
 * The models of the services, as the program asks each service for them with the key that a
 * session would send. The page holds one of these for the field and the settings, and asks a
 * service no more often than it must. No key comes this way.
 */
export function useModels() {
  const [known, setKnown] = useState<Record<string, Models>>({});
  const now = useRef(known);
  now.current = known;
  // The asks of each service, by number. Only the answer to the last one counts.
  const asked = useRef(new Map<string, number>());

  /** Ask a service for its models, whatever is known of them. */
  const ask = useCallback(async (service: string) => {
    const bridge = window.columbina;
    if (!bridge) return;
    const number = (asked.current.get(service) ?? 0) + 1;
    asked.current.set(service, number);
    setKnown(current => {
      const before = current[service];
      return { ...current, [service]: { kind: 'asking', models: before?.kind === 'failed' ? [] : before?.models ?? [] } };
    });
    let found: Models;
    try {
      const checked = checkedOf(await bridge.listModels(service));
      found = checked.kind === 'models'
        ? { kind: 'listed', models: checked.models, at: Date.now() }
        : { kind: 'failed', failure: checked.failure, said: checked.said, at: Date.now() };
    } catch (problem) {
      found = { kind: 'failed', failure: 'unknown', said: said(problem), at: Date.now() };
    }
    if (asked.current.get(service) !== number) return;
    setKnown(current => ({ ...current, [service]: found }));
  }, []);

  /**
   * Ask a service for its models where nothing is known of them, the last ask failed, or what it
   * listed is older than a few minutes. Not while it is being asked.
   */
  const freshen = useCallback((service: string) => {
    const known = now.current[service];
    if (known?.kind === 'asking') return;
    if (known?.kind === 'listed' && Date.now() - known.at < MODELS_FRESH_MS) return;
    void ask(service);
  }, [ask]);

  const of = useCallback((service: string): Models | null => known[service] ?? null, [known]);

  return { of, ask, freshen };
}

export type ServiceModels = ReturnType<typeof useModels>;
