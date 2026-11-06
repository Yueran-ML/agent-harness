import { useEffect, useState } from 'react';
import type { AudioPresetPayload } from '../engine';
import { useFairlightText } from '../fairlight/words';
type Pair = { selected: 'A' | 'B'; A: AudioPresetPayload; B: AudioPresetPayload };
const stores = new WeakMap<object, Map<string, Pair>>();
export function clearAudioComparisons(bridge: object, prefix: string, identities: string[]) {
  const store = stores.get(bridge); if (!store) return;
  for (const key of store.keys()) if (key.startsWith(prefix) && !identities.includes(key)) store.delete(key);
}
export function ABCompare({ bridge, identity, value, disabled, apply }: { bridge: object; identity: string; value: AudioPresetPayload; disabled: boolean; apply: (value: AudioPresetPayload) => void }) {
  const t = useFairlightText(); let store = stores.get(bridge); if (!store) { store = new Map(); stores.set(bridge, store); }
  let pair = store.get(identity); if (!pair) { pair = { selected: 'A', A: structuredClone(value), B: structuredClone(value) }; store.set(identity, pair); }
  const entry = pair, [, redraw] = useState(0);
  const settings = JSON.stringify(value);
  useEffect(() => { entry[entry.selected] = structuredClone(value); }, [settings, identity]);
  const select = (side: 'A' | 'B') => { if (side === entry.selected || disabled) return; entry[entry.selected] = structuredClone(value); const next = structuredClone(entry[side]); entry.selected = side; apply(next); redraw(n => n + 1); };
  return <div className="audio-ab" role="group" aria-label={t('A/B comparison')}><button type="button" disabled={disabled} aria-pressed={entry.selected === 'A'} onClick={() => select('A')}>A</button><button type="button" disabled={disabled} aria-pressed={entry.selected === 'B'} onClick={() => select('B')}>B</button><button type="button" aria-label={t('Copy comparison')} onClick={() => { entry[entry.selected === 'A' ? 'B' : 'A'] = structuredClone(value); redraw(n => n + 1); }}>{entry.selected === 'A' ? 'A → B' : 'B → A'}</button></div>;
}
