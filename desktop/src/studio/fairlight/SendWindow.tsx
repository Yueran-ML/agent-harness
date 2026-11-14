import { Sheet } from '../../app/Sheet';
import { Fader } from '../audio/Fader';
import { PanFields } from '../audio/PanControl';
import { useRoutingParameter } from '../audio/useRoutingParameter';
import type { ProcessorProps } from '../audio/useProcessor';
import { useFairlightText } from './words';
function SendRow({ context, stripKey, bus }: { context: ProcessorProps; stripKey: string; bus: string }) {
  const control = useRoutingParameter(context, stripKey, bus), t = useFairlightText();
  const target = context.timeline.buses?.find(row => row.id === bus);
  return <article><header><strong style={{ color: target?.color }}>{target?.name ?? bus}</strong>{(['on', 'pre'] as const).map(field => <label key={field}
    data-automation-state={field === 'on' ? control.status(field) ?? undefined : undefined}
    onContextMenu={field === 'on' ? event => { event.preventDefault(); control.show(field); } : undefined}>
    <input type="checkbox" checked={control.reading(field) >= .5} disabled={!control.writable(field)} onChange={event => control.commit({ [field]: Number(event.target.checked) })} />{t(field === 'on' ? 'On' : 'Pre')}</label>)}</header>
    <div className="send-controls"><Fader label={`${t('Send Level')} ${target?.name ?? bus}`} value={control.reading('level_db')} disabled={!control.writable('level_db')}
      onStart={() => control.start(['level_db'])} isActive={() => control.active(['level_db'])} onChange={value => control.change({ level_db: Math.max(-100, Math.min(10, value)) })}
      onEnd={() => control.end(['level_db'])} onCancel={() => control.cancel(['level_db'])} onCommit={value => control.commit({ level_db: value })} />
      <PanFields control={control} prefix="pan." formats={target ? [target.format] : []} /></div>{control.problem && <p role="alert">{control.problem}</p>}</article>;
}
export function SendWindow({ context, stripKey, onClose }: { context: ProcessorProps; stripKey: string; onClose: () => void }) {
  const t = useFairlightText();
  const owner = stripKey.startsWith('track:') ? context.timeline.tracks.find(row => row.id === stripKey.slice(6)) : context.timeline.buses?.find(row => row.id === stripKey.slice(4));
  const sends = owner && 'strip' in owner ? owner.strip?.sends ?? [] : [];
  return <Sheet title={`${t('Bus Sends')} · ${owner?.name ?? ''}`} onClose={onClose} wide><div className="send-window">
    {sends.map(send => <SendRow key={send.bus} context={context} stripKey={stripKey} bus={send.bus} />)}
    {!sends.length && <p>{t('No sends')}</p>}</div></Sheet>;
}
