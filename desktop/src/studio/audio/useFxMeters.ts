import { useEffect, useState } from 'react';
import { callEngine, type FxMeters } from '../engine';
import type { ProcessorProps } from './useProcessor';

type Listener = (value: FxMeters | null) => void;
interface Poll { listeners: Set<Listener>; value: FxMeters | null; stop: () => void }
const bridges = new WeakMap<object, Map<string, Poll>>();
/** Visible panels share one bounded poll; the last unmount releases it and late replies. */
function subscribe(props: ProcessorProps, listener: Listener) {
  let polls = bridges.get(props.bridge); if (!polls) { polls = new Map(); bridges.set(props.bridge, polls); }
  const key = `${props.session}/${props.timeline.id}/${props.revision}`;
  let poll = polls.get(key);
  if (!poll) {
    let alive = true, timer: ReturnType<typeof setTimeout> | undefined;
    poll = { listeners: new Set(), value: null, stop: () => { alive = false; clearTimeout(timer); } };
    const current = poll;
    const publish = (value: FxMeters | null) => { current.value = value; current.listeners.forEach(fn => fn(value)); };
    const tick = async () => {
      if (document.hidden) { publish(null); timer=setTimeout(()=>void tick(),100); return; }
      try { const result = await callEngine(props.bridge, 'audio.fx_meters', { timeline: props.timeline.id }, { expectedSession: props.session });
        if (alive) publish(result?.playing && result.timeline === props.timeline.id ? result : null);
      } catch { if (alive) publish(null); }
      if (alive) timer = setTimeout(() => void tick(), 100);
    };
    polls.set(key, poll); void tick();
  }
  poll.listeners.add(listener); listener(poll.value);
  return () => { poll!.listeners.delete(listener); if (!poll!.listeners.size) { poll!.stop(); polls!.delete(key); } };
}
export function useFxMeters(props: ProcessorProps) {
  const [value, setValue] = useState<FxMeters | null>(null);
  useEffect(() => { setValue(null); return subscribe(props, setValue); }, [props.bridge, props.session, props.timeline.id, props.revision]);
  return value;
}
