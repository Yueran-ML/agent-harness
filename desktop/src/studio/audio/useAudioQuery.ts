import { useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';

/** Exact comparison of the small JSON model inputs used by an audio query. */
function sameInputs(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const left = a as Record<string, unknown>, right = b as Record<string, unknown>;
  const keys = Object.keys(left);
  return keys.length === Object.keys(right).length
    && keys.every(key => Object.hasOwn(right, key) && sameInputs(left[key], right[key]));
}

interface Entry {
  inputs: unknown;
  users: number;
  result: Promise<unknown>;
}
const queries = new WeakMap<object, Set<Entry>>();

/** Share mounted consumers' queries; unrelated revisions reuse the same model inputs.
 * Inputs include the session, owner and query kind. No results survive the last consumer.
 */
export function useAudioQuery<T extends { revision: number }>(
  bridge: Pick<EngineBridge, 'call'>,
  inputs: unknown,
  revision: number,
  enabled: boolean,
  request: () => Promise<T>,
) {
  const stable = useRef<unknown>(undefined);
  if (!sameInputs(stable.current, inputs)) stable.current = structuredClone(inputs);
  const identity = stable.current;
  const [state, setState] = useState<{ bridge: object; identity: unknown; data?: T; problem?: string }>();
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    let entries = queries.get(bridge);
    if (!entries) { entries = new Set(); queries.set(bridge, entries); }
    let entry = [...entries].find(row => sameInputs(row.inputs, identity));
    if (!entry) {
      entry = { inputs: identity, users: 0, result: request().then(result => {
        if (result.revision < revision) throw new Error('The audio query returned an older project revision.');
        return result;
      }) };
      entries.add(entry);
    }
    const used = entry, cache = entries;
    used.users++;
    void used.result.then(data => {
      if (live) setState({ bridge, identity, data: data as T });
    }).catch(error => {
      cache.delete(used);
      if (live) setState({ bridge, identity, problem: String(error) });
    });
    return () => {
      live = false;
      used.users--;
      // Allow the same commit's effects (including another consumer) to reattach first.
      queueMicrotask(() => { if (!used.users) cache.delete(used); });
    };
  }, [bridge, identity, revision, enabled]);
  const valid = enabled && state?.bridge === bridge && state.identity === identity;
  return { data: valid ? state.data : undefined, problem: valid ? state.problem ?? null : null };
}
