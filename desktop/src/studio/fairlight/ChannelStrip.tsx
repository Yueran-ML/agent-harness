import { useAutomationTouch } from './AutomationTouch';
import type { AutomationArm, LanesOf } from '../engine';
import { laneValue } from './automation';
import { useState, type PointerEvent } from 'react';
import { createPortal } from 'react-dom';
import type { MeterChannel } from '../engine';
import { NameField } from '../media/NameField';
import { Fader } from '../audio/Fader';
import { Meter } from '../audio/Meter';
import { faderReading, formatDb, type MixerStrip, type Modifiers } from '../audio/mixerModel';
import { useFairlightText } from './words';
import type { ProcessorProps } from '../audio/useProcessor';
import { defaultDynamics, defaultEq, ORDERINGS, processorWritable, type ProcessorModule } from '../audio/processorModel';
import { ProcessorThumbnail } from '../audio/ProcessorThumbnail';
import { PathSettings } from './PathSettings';
import { RouteButtons } from './RouteButtons';
import { PanControl } from '../audio/PanControl';
import { SendWindow } from './SendWindow';
import { VcaAssignment } from './VcaAssignment';
import { useVcaGain } from './VcaStrip';
import { GroupLabel } from './TrackGroups';

import { InsertSlots } from './InsertSlots';
import { DuckerPanel } from '../audio/DuckerPanel';
import { StripCopy } from '../audio/StripCopy';

export function ChannelStrip({ strip, seconds, value, selected, meters, shadow, onSelect, onRename, onStart, onChange, onEnd, onCancel, onCommit, isActive, onToggle, onSweep, onEnter, onArm, processorContext, onProcessor, onStripSetting, onReorder, selectedKeys = [], compact = false }: {
  onReorder?: (keys: string[], before: string) => void; selectedKeys?: readonly string[];
  strip: MixerStrip; seconds: number; value?: number; selected: boolean; meters?: MeterChannel[]; shadow: boolean;
  onSelect: (modifiers: Modifiers) => void; onRename: (name: string) => Promise<boolean>;
  onStart: (modifiers: Modifiers) => boolean; onChange: (value: number) => void; onEnd: () => void; onCancel: () => void;
  onCommit: (value: number, modifiers?: Modifiers) => void;
  isActive: () => boolean;
  onToggle: (field: 'mute' | 'solo' | 'solo_safe', modifiers: Modifiers) => void;
  onSweep: (field: 'mute' | 'solo' | 'solo_safe', event: PointerEvent<HTMLButtonElement>) => void;
  onArm?: (arm: AutomationArm, modifiers: Modifiers) => void;
  onEnter: (field: 'mute' | 'solo', event: PointerEvent<HTMLButtonElement>) => void;
  processorContext: ProcessorProps; onProcessor: (module: ProcessorModule) => void;
  onStripSetting: (path: string, value: unknown, modifiers: Modifiers) => void;
  compact?: boolean;
}) {
  const t = useFairlightText(); const [renaming, setRenaming] = useState(false), [pathOpen, setPathOpen] = useState<{ x: number; y: number } | null>(null);
  const automation=useAutomationTouch(); const of={ [strip.kind]:strip.id } as LanesOf; const target={of,path:'fader_db'};
  const recording=Boolean(automation?.enabled&&automation.recording), trim=Boolean(recording&&automation?.trim&&strip.arm!=='safe'&&strip.arm!=='off'&&(strip.arm===true||processorContext.timeline.audio?.record?.touch!=='off'));
  const [menu,setMenu]=useState<'arm'|'fader'|null>(null);
  const [sendsOpen, setSendsOpen] = useState(false);
  const reading = faderReading(strip, seconds, automation?.enabled??true), model = strip.strip;
  const inheritedGain = useVcaGain(processorContext.timeline, model?.vcas ?? [], seconds);
  const baseFader=trim?0:reading.automated?reading.value:value??reading.value;
  const faderValue=automation?.reading(target,baseFader)??baseFader;
  const baseMute=(automation?.enabled??true)&&strip.arm!=='off'?laneValue(strip.lanes?.mute,seconds,strip.mute?1:0)>=.5:strip.mute;
  const muted=(automation?.reading({of,path:'mute'},Number(baseMute),{kind:'switch',scale:'linear'})??Number(baseMute))>=.5;
  const instantMute=()=>automation?.instant({of,path:'mute'},muted?0:1);
  const [duckerOpen, setDuckerOpen] = useState(false);
  return <article className="mixer-strip" data-mixer-strip={strip.key} data-selected={selected || undefined} data-disabled={!strip.enabled || undefined}
    data-recording={automation?.status(target)==='write'||undefined} data-preview={automation?.status(target)==='preview'||undefined} data-trim={trim||undefined} aria-label={strip.name} onClick={event => { if (event.target === event.currentTarget) onSelect(event); }}
    onDragOver={event => { if (event.dataTransfer.types.includes('application/x-columbina-mixer-order')) event.preventDefault(); }}
    onDrop={event => { const raw = event.dataTransfer.getData('application/x-columbina-mixer-order'); if (!raw) return; event.preventDefault(); try { const keys: unknown = JSON.parse(raw); if (Array.isArray(keys) && keys.every(k => typeof k === 'string')) onReorder?.(keys, strip.key); } catch { /* Ignore unrelated drags. */ } }}>
    <div className="mixer-strip-color" style={{ background: strip.color }}><StripCopy context={processorContext}>{strip.kind === 'track' ? 'A' : 'B'}{strip.number}</StripCopy></div>
    <dl className="mixer-summary">
      <div data-core data-section="input"><dt>{t('Input')}</dt><dd><button type="button" aria-label={`${t('Path Settings')} ${strip.name}`} onClick={event => {
        const rect = event.currentTarget.getBoundingClientRect(); setPathOpen(pathOpen ? null : { x: Math.max(8, Math.min(window.innerWidth - 220, rect.left)), y: Math.max(8, Math.min(window.innerHeight - 240, rect.bottom)) });
      }}>{t('Audio')}</button></dd></div>
      {strip.kind === 'track' && <div data-section="trackfx"><dt>{t('Track FX')}</dt><dd><button type="button" onClick={() => setDuckerOpen(true)}>{t('Ducker')}</button></dd></div>}
      <div data-core data-section="order"><dt>{t('Order')}</dt><dd><select aria-label={`${t('Order')} ${strip.name}`} disabled={strip.locked} value={(model?.order ?? ORDERINGS[0]).join(',')}
        onChange={event => onStripSetting('order', event.target.value.split(','), {})}>{ORDERINGS.map(order => <option key={order.join(',')} value={order.join(',')}>{order.map(s => s === 'fx' ? 'FX' : s === 'eq' ? 'EQ' : 'DY').join(' › ')}</option>)}</select></dd></div>
      <div data-core data-insert-slots data-section="effects"><dt>{t('Effects')}</dt><dd><InsertSlots {...processorContext} /></dd></div>
      <div data-core data-section="effectsIn"><dt>{t('Effects In')}</dt><dd><button type="button" className="effects-in" aria-label={`${t('Effects In')} ${strip.name}`} aria-pressed={model?.inserts_on !== false}
        disabled={recording||strip.locked||(automation?.enabled!==false&&!processorWritable(strip, 'inserts_on'))} onClick={event => onStripSetting('inserts_on', model?.inserts_on === false, event)}>{t(model?.inserts_on === false ? 'Off' : 'On')}</button></dd></div>
      <div data-core data-section="dynamics"><dt>{t('Dynamics')}</dt><dd><ProcessorThumbnail name={strip.name} module="dynamics" openOnClick={compact} on={model?.dynamics?.on ?? false} disabled={strip.locked||(!recording&&automation?.enabled!==false&&!processorWritable(strip, 'dynamics'))}
        settings={model?.dynamics ?? defaultDynamics()} onPaste={(settings, modifiers) => onStripSetting('dynamics', settings, modifiers)}
        context={processorContext} showCurve={Boolean(model?.dynamics)} onToggle={modifiers => onStripSetting('dynamics', { on: !model?.dynamics?.on }, modifiers)} onOpen={() => onProcessor('dynamics')} /></dd></div>
      <div data-core data-section="eq"><dt>EQ</dt><dd><ProcessorThumbnail name={strip.name} module="eq" openOnClick={compact} on={model?.eq?.on ?? false} disabled={strip.locked||(!recording&&automation?.enabled!==false&&!processorWritable(strip, 'eq'))}
        settings={model?.eq ?? defaultEq()} onPaste={(settings, modifiers) => onStripSetting('eq', settings, modifiers)}
        context={processorContext} showCurve={Boolean(model?.eq)} onToggle={modifiers => onStripSetting('eq', { on: !model?.eq?.on }, modifiers)} onOpen={() => onProcessor('eq')} /></dd></div>
      <div data-section="sends"><dt>{t('Sends')}</dt><dd><RouteButtons context={processorContext} strip={strip} field="sends" onOpen={() => setSendsOpen(true)} /></dd></div>
      <div data-section="pan"><dt>{t('Pan')}</dt><dd><PanControl context={processorContext} stripKey={strip.key} /></dd></div>
      <div data-section="outputs"><dt>{t('Outputs')}</dt><dd><RouteButtons context={processorContext} strip={strip} field="outputs" onOpen={() => {}} /></dd></div>
      <div data-section="vca"><dt>VCA</dt><dd><VcaAssignment context={processorContext} of={of} selected={processorContext.selected} /></dd></div>
    </dl>
    {strip.kind==='track'&&<GroupLabel track={strip.id}/>}
    {pathOpen && createPortal(<PathSettings context={processorContext} stripKey={strip.key} anchor={pathOpen} onClose={() => setPathOpen(null)} />, document.body)}
    {sendsOpen && <SendWindow context={processorContext} stripKey={strip.key} onClose={() => setSendsOpen(false)} />}
    {duckerOpen && createPortal(<aside role="dialog" aria-label={`${t('Ducker')} ${strip.name}`} className="processor-window" style={{ left: 100, top: 100 }}><button type="button" aria-label={t('Close processor')} onClick={() => setDuckerOpen(false)}>×</button><DuckerPanel {...processorContext} /></aside>, document.body)}
    {renaming ? <NameField name={strip.name} onCancel={() => setRenaming(false)} onCommit={onRename} />
      : <button type="button" className="mixer-name" draggable onDragStart={event => event.dataTransfer.setData('application/x-columbina-mixer-order', JSON.stringify(selectedKeys.includes(strip.key) ? selectedKeys : [strip.key]))} onClick={event => onSelect(event)} onDoubleClick={() => { if (!strip.locked) setRenaming(true); }}>{strip.name}</button>}
    {automation?.enabled&&<div className="mixer-arm"><button disabled={strip.locked} aria-pressed={strip.arm===true} onClick={e=>onArm?.(strip.arm!==true,e)} onContextMenu={e=>{e.preventDefault();setMenu('arm');}}>{t('Arm')}</button>
      {menu==='arm'&&<div role="menu">{(['safe','off'] as const).map(arm=><button key={arm} onClick={e=>{onArm?.(arm,e);setMenu(null);}}>{t(arm==='safe'?'Safe':'Bypass')}</button>)}<button onClick={()=>setMenu(null)}>{t('Cancel')}</button></div>}</div>}
    <div className="mixer-switches">{(['solo', 'mute'] as const).map(field => <button key={field} type="button" disabled={strip.locked}
      aria-label={`${t(field)} ${strip.name}`} aria-pressed={field==='mute'?muted:strip[field]} data-safe={field === 'solo' && strip.safe || undefined}
      title={field === 'solo' ? `${t('Solo Safe')}: Ctrl+Shift` : t('mute')}
      onPointerDown={event => {if(field==='mute'&&recording){event.preventDefault();instantMute();}else onSweep(field === 'solo' && event.ctrlKey && event.shiftKey ? 'solo_safe' : field, event);}}
      onPointerEnter={event => {if(!recording)onEnter(field, event);}}
      onClick={event => { if(event.detail===0&&field==='mute'&&recording){instantMute();return;} if (event.detail === 0) onToggle(field === 'solo' && event.ctrlKey && event.shiftKey ? 'solo_safe' : field, event); }}>{field === 'solo' ? strip.safe ? 'S·' : 'S' : 'M'}</button>)}</div>
    <div className="mixer-fader-meter" onContextMenu={event=>{if(automation){event.preventDefault();setMenu('fader');}}}>
      {menu==='fader'&&<div role="menu"><button onClick={()=>{automation?.show(target);setMenu(null);}}>{t('Show automation in timeline')}</button><button onClick={()=>setMenu(null)}>{t('Cancel')}</button></div>}
      <Fader label={`${t('Fader')} ${strip.name}`} valueLabel={`${t('Fader value')} ${strip.name}`} value={faderValue} displayOffset={trim ? 0 : inheritedGain}
        disabled={strip.locked} readOnly={reading.automated&&!recording} hasLane={reading.hasLane} onStart={m=>recording?automation!.start(target,faderValue):onStart(m)}
        onChange={v=>{if(recording)automation!.changeBatch([{target,value:v}]);else onChange(v);}}
        onEnd={()=>{if(recording){if(automation!.active(target))automation!.end();}else onEnd();}}
        onCancel={()=>{if(automation?.active(target))automation.end();onCancel();}}
        onCommit={(v,m)=>{if(recording)automation!.instant(target,v);else onCommit(v,m);}} isActive={()=>recording?automation!.active(target):isActive()} />
      <Meter label={strip.name} channels={strip.channels} values={meters} disabled={!strip.enabled} shadow={shadow} />
    </div>
    {reading.automated && <small className="mixer-automation" title={t('Automation controls playback. Static settings do not replace the lane.')}>{t('Automation controls playback')}<br />{t('Base value')}: {formatDb(strip.db)} dB</small>}
  </article>;
}
