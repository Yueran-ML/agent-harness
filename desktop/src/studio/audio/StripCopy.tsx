import { useState, type ReactNode } from 'react';
import { stripClipboard } from './audioClipboard';
import type { ProcessorProps } from './useProcessor';
import { copyStripSettings } from './effectModel';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { targetKey } from './processorModel';
import { useFailureText, useFairlightText } from '../fairlight/words';

export function StripCopy({ context, children }: { context: ProcessorProps; children?: ReactNode }) {
  const t = useFairlightText(), [open, setOpen] = useState(false), [problem, setProblem] = useState<string | null>(null);
  const failure = useFailureText();
  const gesture = useAudioGesture({ ...context, identity: `${context.timeline.id}/${targetKey(context.target)}/copy`, locked: Boolean(context.automation?.recording), onFailure: error => setProblem(failure(error)) });
  const clipboard=stripClipboard.use();
  const source = clipboard?.bridge === context.bridge && clipboard?.session === context.session && clipboard.timeline === context.timeline.id ? clipboard.target : null;
  const changes = source && !('clip' in context.target) ? copyStripSettings(context.timeline, source, context.target) : [];
  return <span onContextMenu={event => { event.preventDefault(); setOpen(true); }}>{children}<button type="button" aria-label={t('Strip settings')} onClick={() => setOpen(old => !old)}>…</button>{problem && <small role="alert">{problem}</small>}{open && <span className="mixer-layout-menu">
    <button type="button" onClick={() => { if (!('clip' in context.target)) stripClipboard.set({ bridge:context.bridge, session: context.session, timeline: context.timeline.id, target: context.target }); }}>{t('Copy strip settings')}</button>
    <button type="button" disabled={!changes.length || context.automation?.recording} onClick={() => { gesture.commit(changes); setOpen(false); }}>{t('Paste strip settings')}</button>
  </span>}</span>;
}
