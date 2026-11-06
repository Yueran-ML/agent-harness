import { useEffect, useRef, useState } from 'react';
import { Sheet } from '../../app/Sheet';
import type { EngineBridge } from '../../bridge';
import type { MenuItem } from '../color/ColorMenu';
import { callEngine, type Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import type { ClipAttribute } from '../timeline/commands';
import { normalizeTargets } from './NormalizeDialog';

type Bridge = Pick<EngineBridge, 'call'>;
type Source = { session: number; timeline: string; clip: string };
type Request = { session: number; timeline: string; revision: number; clips: string[]; from?: string };
// The same project can move between Edit and Fairlight without losing its attribute source.
const sources = new WeakMap<Bridge, Source>();
const attributes: { id: ClipAttribute; label: 'Volume' | 'Plugins' | 'Equalizer' }[] = [
  { id: 'volume', label: 'Volume' }, { id: 'plugins', label: 'Plugins' }, { id: 'equalizer', label: 'Equalizer' },
];
const soundIds = (timeline: Timeline | null) => new Set(timeline?.tracks.flatMap(track => track.kind === 'audio' ? track.clips.map(clip => clip.id) : []) ?? []);

/** Source is an existing clip ID, as paste_attributes requires; curve mapping stays in the model. */
export function useClipAttributes({ bridge, session, revision, timeline, selection }: {
  bridge: Bridge; session: number; revision: number; timeline: Timeline | null; selection: readonly string[];
}) {
  const t = useFairlightText();
  const [opened, setOpened] = useState<Request | null>(null);
  const [, changed] = useState(0);
  const source = sources.get(bridge);
  const sounds = soundIds(timeline);
  const validSource = source?.session === session && source.timeline === timeline?.id && sounds.has(source.clip) ? source.clip : null;
  useEffect(() => {
    const saved = sources.get(bridge);
    if (saved && (saved.session !== session || saved.timeline !== timeline?.id || !soundIds(timeline).has(saved.clip))) sources.delete(bridge);
  }, [bridge, session, timeline]);
  useEffect(() => { setOpened(null); }, [session, timeline?.id]);
  const copy = (ids: readonly string[]) => {
    const audio = [...new Set(ids)].filter(id => sounds.has(id));
    if (!timeline || audio.length !== 1) { sources.delete(bridge); changed(n => n + 1); return false; }
    sources.set(bridge, { session, timeline: timeline.id, clip: audio[0] }); changed(n => n + 1); return true;
  };
  const items = (context: string): MenuItem[] => {
    const requested = selection.includes(context) ? selection : [context];
    const clips = normalizeTargets(timeline, selection, context);
    const open = (from?: string) => { if (timeline && clips.length) setOpened({ session, timeline: timeline.id, revision, clips, from }); };
    return [
      { id: 'copy-attributes', label: t('Copy Attributes'), keys: 'Ctrl+C', disabled: requested.filter(id => sounds.has(id)).length !== 1, run: () => { copy(requested); } },
      { id: 'paste-attributes', label: t('Paste Attributes'), disabled: !validSource || !clips.length, run: () => { if (validSource) open(validSource); } },
      { id: 'remove-attributes', label: t('Remove Attributes'), disabled: !clips.length, run: () => open() },
    ];
  };
  return { copy, items, dialog: opened && opened.session === session && opened.timeline === timeline?.id
    ? <ClipAttributesDialog bridge={bridge} request={opened} revision={revision} onClose={() => setOpened(null)} /> : null };
}

function ClipAttributesDialog({ bridge, request, revision, onClose }: {
  bridge: Bridge; request: Request; revision: number; onClose: () => void;
}) {
  const t = useFairlightText();
  const [items, setItems] = useState<ClipAttribute[]>(attributes.map(item => item.id));
  const [pending, setPending] = useState(false), [problem, setProblem] = useState<string | null>(null);
  const busy = useRef(false), live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  const stale = request.revision !== revision;
  const apply = async () => {
    if (busy.current || stale || !items.length) return;
    busy.current = true; setPending(true); setProblem(null);
    const op = request.from
      ? { edit: 'paste_attributes' as const, timeline: request.timeline, from: request.from, to: request.clips, items }
      : { edit: 'remove_attributes' as const, timeline: request.timeline, clips: request.clips, items };
    try {
      await callEngine(bridge, 'transaction.apply', { revision: request.revision, ops: [op] }, { expectedSession: request.session });
      if (live.current) onClose();
    } catch (error) { if (live.current) setProblem(error instanceof Error ? error.message : String(error)); }
    finally { busy.current = false; if (live.current) setPending(false); }
  };
  return <Sheet title={t(request.from ? 'Paste Attributes' : 'Remove Attributes')} onClose={() => { if (!pending) onClose(); }} foot={<>
    <button type="button" className="btn" data-first disabled={pending} onClick={onClose}>{t('Cancel')}</button>
    <button type="button" className="btn btn-main" disabled={pending || stale || !items.length} onClick={() => void apply()}>{t('Apply')}</button>
  </>}>
    <fieldset disabled={pending || stale}>{attributes.map(item => <label key={item.id} style={{ display: 'block' }}>
      <input type="checkbox" checked={items.includes(item.id)} onChange={event => setItems(old => event.target.checked ? [...old, item.id] : old.filter(id => id !== item.id))} />{t(item.label)}
    </label>)}</fieldset>
    {stale && <p role="alert">{t('The timeline changed. Reopen this dialog.')}</p>}
    {problem && <p role="alert">{problem}</p>}
  </Sheet>;
}
