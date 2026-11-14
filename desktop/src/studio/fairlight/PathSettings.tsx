import { useAutomationTouch } from './AutomationTouch';
import { ownerOf, laneValue } from './automation';
import { useState } from 'react';
import { useOverlay } from '../../app/overlay';
import { Knob } from '../audio/Knob';
import type { ProcessorProps } from '../audio/useProcessor';
import { processorChanges, processorWritable } from '../audio/processorModel';
import { mixerStrips } from '../audio/mixerModel';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { useFailureText, useFairlightText } from './words';
import { useShowAutomation } from './ShowAutomationMenu';

export function PathSettings({ context, stripKey, anchor, onClose }: { context: ProcessorProps; stripKey: string; anchor: { x: number; y: number }; onClose: () => void }) {
  useOverlay();
  const t = useFairlightText(), strip = mixerStrips(context.timeline).find(s => s.key === stripKey)!;
  const failure = useFailureText(), showMenu = useShowAutomation();
  const [problem, setProblem] = useState(''), [preview, setPreview] = useState<number | null>(null);
  const automation=useAutomationTouch(), recording=Boolean(automation?.enabled&&automation.recording), of=ownerOf(stripKey), target={of,path:'trim_db'};
  const baseTrim=strip.strip?.trim_db??0;
  const trim=(automation?.enabled??true)&&strip.arm!=='off'?laneValue(strip.lanes?.trim_db,context.seconds??0,baseTrim):baseTrim;
  const switched=(path:'invert'|'inserts_on')=>{
    const base=path==='invert'?Boolean(strip.strip?.invert):strip.strip?.inserts_on!==false;
    const baseline=(automation?.enabled??true)&&strip.arm!=='off'?laneValue(strip.lanes?.[path],context.seconds??0,Number(base)):Number(base);
    return (automation?.reading({of,path},baseline,{kind:'switch',scale:'linear'})??baseline)>=.5;
  };
  const writable=(path:string)=>!strip.locked&&(recording||automation?.enabled===false||processorWritable(strip,path));
  const gesture = useAudioGesture({ ...context, identity: `${context.timeline.id}/${stripKey}/path`, locked: strip.locked,
    onFailure: error => setProblem(failure(error)) });
  const changes = (path: string, value: unknown) => processorChanges(context.timeline.id, [automation?.enabled===false?{...strip,arm:'off'}:strip], path, value);
  return <section data-audio-owner={stripKey} className="path-settings" style={{ left: anchor.x, top: anchor.y }} aria-label={t('Path Settings')} onKeyDown={event => { if (event.key === 'Escape') { gesture.cancel(); setPreview(null); event.stopPropagation(); } }}>
    <header>{strip.name} · {t('Path Settings')}<button type="button" aria-label={t('Close processor')} onClick={onClose}>×</button></header>
    {problem && <p role="alert">{problem}</p>}
    <Knob path="trim_db" label={t('Trim')} value={recording?automation!.reading(target,trim):gesture.active()?preview??trim:trim} min={-20} max={20} fallback={0} unit="dB" disabled={!writable('trim_db')}
      onStart={()=>recording?automation!.start(target,trim):gesture.start(changes('trim_db',baseTrim))} isActive={()=>recording?automation!.active(target):gesture.active()}
      onChange={value=>{setPreview(value);if(recording)automation!.changeBatch([{target,value}]);else gesture.preview(changes('trim_db',value));}}
      onEnd={()=>{if(recording){if(automation!.active(target))automation!.end();}else gesture.end();setPreview(null);}}
      onCancel={()=>{if(automation?.active(target))automation.end();gesture.cancel();setPreview(null);}}
      onCommit={value=>{if(recording)automation!.instant(target,value);else gesture.commit(changes('trim_db',value));}} />
    {showMenu.menu}
    <label data-automation-state={automation?.status({of,path:'invert'})??undefined} onContextMenu={showMenu.open({of,path:'invert'})}>
      <input type="checkbox" checked={switched('invert')} disabled={!writable('invert')} onChange={event=>{if(recording)automation!.instant({of,path:'invert'},Number(event.target.checked));else gesture.commit(changes('invert',event.target.checked));}} />{t('Polarity')}</label>
    <label><input type="checkbox" checked={switched('inserts_on')} disabled={recording||!writable('inserts_on')} onChange={event=>gesture.commit(changes('inserts_on',event.target.checked))} />{t('Insert')}</label>
  </section>;
}
