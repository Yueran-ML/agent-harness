// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { AutomationTouchProvider } from '../src/studio/fairlight/AutomationTouch';
import { defaultRecord } from '../src/studio/fairlight/automation';
import { EffectPanel } from '../src/studio/audio/EffectPanel';
import { DuckerPanel } from '../src/studio/audio/DuckerPanel';
import { InsertSlots } from '../src/studio/fairlight/InsertSlots';
import { Inspector } from '../src/studio/inspector/Inspector';
import { Inspector as FairlightInspector } from '../src/studio/fairlight/Inspector';
import { Mixer } from '../src/studio/fairlight/Mixer';
import { IndexTracks } from '../src/studio/fairlight/IndexTracks';
import { mixerOrder, moveMixerItems, useMixerLayout } from '../src/studio/audio/mixerLayout';
import { defaultEffect, defaultDucker } from '../src/studio/audio/effectModel';
import type { AutomationParameter, Timeline } from '../src/studio/engine';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(()=>{likeABrowser();vi.stubGlobal('PointerEvent',MouseEvent);});
afterEach(()=>{cleanup();vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();});
const settle=()=>act(async()=>{for(let i=0;i<20;i++)await Promise.resolve();});
const parameter=(path:string,value=0,kind:'continuous'|'switch'='continuous'):AutomationParameter=>({path,value,kind,scale:'linear',min:-100,max:1000});
const parameters=[parameter('insert.fx.input_db',-6),parameter('insert.fx.on',1,'switch'),parameter('track_fx.amount_db',2.7),parameter('track_fx.on',0,'switch')];
const song=():Timeline=>({...timeline([]),tracks:[{id:'a',kind:'audio',name:'Voice',enabled:true,locked:false,clips:[],transitions:[],automation_arm:true,
  strip:{inserts:[{id:'fx',on:true,effect:{...defaultEffect('limiter'),input_db:-6}}],track_fx:defaultDucker()} as any}],audio:{record:{...defaultRecord(),touch:'latch'}} as Timeline['audio']});
function wire(catalogue=parameters){
  const call=vi.fn(async(method:string,params:any)=>({ok:true as const,result:method==='automation.parameters'?{revision:7,of:params.of,parameters:catalogue}:method==='audio.fx_meters'?{timeline:'t1',playing:false,generation:0,readings:[]}:method==='audio.eq_response'?{revision:7,sample_rate:48000,frequencies:params.frequencies,gain_db:params.frequencies.map(()=>0)}:{revision:8,done:[]}}));
  return {call, onEvent: vi.fn(() => () => {})};
}
const writes=(bridge:ReturnType<typeof wire>,method='automation.touch')=>bridge.call.mock.calls.filter(c=>c[0]===method).map(c=>c[1]);
function processor(ducker=false,mode:'preview'|'latch'|'snap'='latch',enabled=true,model=song()){
  const bridge=wire();model.audio!.record={...model.audio!.record,touch:mode==='snap'?'snap':'latch',preview:mode==='preview',mode:'trim'};
  const context={bridge,session:1,revision:7,timeline:model,target:{track:'a'}};
  const element=(on=enabled)=><AutomationTouchProvider {...context} enabled={on} recording playing={mode!=='preview'} show={()=>{}} onFailure={e=>{throw e;}}>
    {ducker?<DuckerPanel {...context}/>:<EffectPanel {...context} index={0}/>}</AutomationTouchProvider>;
  return {...render(element()),bridge,context,element};
}

it.each([false,true])('uses model metadata for recordability and never records unavailable parameters (Ducker=%s)',async ducker=>{
  const s=processor(ducker);await settle();
  expect(screen.getByRole('slider',{name:ducker?'Rise Time':'Ceiling'}).getAttribute('aria-disabled')).toBe('true');
  const input=screen.getByRole('spinbutton',{name:ducker?'Rise Time value':'Ceiling value'});
  fireEvent.change(input,{target:{value:'50'}});fireEvent.keyDown(input,{key:'Enter'});await settle();
  expect(writes(s.bridge)).toEqual([]);expect(writes(s.bridge,'transaction.apply')).toEqual([]);
  expect(s.bridge.call).toHaveBeenCalledWith('automation.parameters',{timeline:'t1',of:{track:'a'}},{expectedSession:1});
});

it.each([['preview',false],['latch',false],['snap',false],['preview',true],['latch',true],['snap',true]] as const)('reads released %s values/status through the Provider (Ducker=%s)',async(mode,ducker)=>{
  vi.useFakeTimers();const s=processor(ducker,mode);await settle();const cap=screen.getByRole('slider',{name:ducker?'Duck Amount':'Input Level'});
  fireEvent.pointerDown(cap,{button:0,clientY:100});fireEvent.pointerMove(cap,{clientY:80});fireEvent.pointerUp(cap);await settle();
  expect(writes(s.bridge).at(-1)).toMatchObject({track:'a',path:ducker?'track_fx.amount_db':'insert.fx.input_db',touching:false,value:ducker?4.7:-4});
  expect(Number(cap.getAttribute('aria-valuenow'))).toBeCloseTo(ducker?4.7:-4);
  expect(cap.getAttribute('data-automation')).toBe(mode==='preview'?'preview':'write');
  await act(async()=>vi.advanceTimersByTimeAsync(400));
  expect(Number(cap.getAttribute('aria-valuenow'))).toBeCloseTo(mode==='snap'?(ducker?2.7:-6):(ducker?4.7:-4));
  expect(cap.getAttribute('data-automation')).toBe(mode==='snap'?null:mode==='preview'?'preview':'write');
  expect(writes(s.bridge,'transaction.apply')).toEqual([]);
});

it.each([false,true])('shows editable static baseline when Automation is bypassed (Ducker=%s)',async ducker=>{
  const t=song();(t.tracks[0] as any).lanes={['insert.fx.input_db']:{points:[{at:{ticks:0,scale:48000},value:9}]},['track_fx.amount_db']:{points:[{at:{ticks:0,scale:48000},value:20}]}};
  const s=processor(ducker,'latch',true,t);await settle();expect(screen.getByRole('slider',{name:ducker?'Duck Amount':'Input Level'}).getAttribute('aria-valuenow')).toBe(ducker?'20':'9');
  s.rerender(s.element(false));await settle();const input=screen.getByRole('spinbutton',{name:ducker?'Duck Amount value':'Input Level value'});
  expect(input).toHaveProperty('disabled',false);expect(input).toHaveProperty('value',ducker?'2.7':'-6');
  fireEvent.change(input,{target:{value:'3'}});fireEvent.keyDown(input,{key:'Enter'});await settle();
  expect(writes(s.bridge)).toEqual([]);expect(writes(s.bridge,'transaction.apply')).toHaveLength(1);
});

it('uses metadata for slot switches, holds repeated Preview toggles, and never falls back when category filtering refuses',async()=>{
  const t=song();t.audio!.record={...t.audio!.record,preview:true};const bridge=wire();const props={bridge,session:1,revision:7,timeline:t,target:{track:'a'}};
  const element=()=> <AutomationTouchProvider {...props} enabled recording playing={false} show={()=>{}} onFailure={e=>{throw e;}}><InsertSlots {...props}/></AutomationTouchProvider>;
  const view=render(element());await settle();const toggle=screen.getByRole('button',{name:'Enable Limiter'});
  fireEvent.click(toggle);await settle();expect(toggle.getAttribute('aria-pressed')).toBe('false');
  fireEvent.click(toggle);await settle();expect(toggle.getAttribute('aria-pressed')).toBe('true');
  expect(writes(bridge).filter(v=>!v.touching).map(v=>v.value)).toEqual([0,1]);
  t.audio!.record.categories={...t.audio!.record.categories,misc:false};view.rerender(element());
  fireEvent.click(toggle);await settle();expect(writes(bridge)).toHaveLength(4);expect(writes(bridge,'transaction.apply')).toEqual([]);
  expect(screen.getByRole('button',{name:'Add effect'})).toHaveProperty('disabled',true);
});

it('rejects Safe provider touches rather than using a supplied legacy adapter or static edits',async()=>{
  const t=song();(t.tracks[0] as any).automation_arm='safe';const bridge=wire(),start=vi.fn(()=>true);
  render(<AutomationTouchProvider bridge={bridge} session={1} revision={7} timeline={t} enabled recording playing show={()=>{}} onFailure={()=>{}}>
    <EffectPanel bridge={bridge} session={1} revision={7} timeline={t} target={{track:'a'}} index={0} automation={{enabled:true,recording:true,trim:false,preview:false,start,change:vi.fn(),end:vi.fn(),instant:vi.fn(()=>true),active:()=>false,show:vi.fn(),live:null}}/>
  </AutomationTouchProvider>);await settle();fireEvent.wheel(screen.getByRole('slider',{name:'Input Level'}),{deltaY:-1});await settle();
  expect(start).not.toHaveBeenCalled();expect(writes(bridge)).toEqual([]);expect(writes(bridge,'transaction.apply')).toEqual([]);
});

it('keeps shared Edit/Cut clip EQ/File controls and exposes real clip Effects writes',async()=>{
  const t=song();(t.tracks[0] as any).clips=[clip('c1',0,25,{effects:[{on:true,effect:{kind:'gain',gain_db:3}}]})];const bridge=wire();
  render(<Inspector bridge={bridge} session={1} revision={7} timeline={t} media={[media]} clips={['c1']} transition={null} frame={12} onSpeed={()=>{}}/>);
  expect(screen.getByRole('button',{name:'Equalizer'})).toBeTruthy();expect(screen.getByRole('tab',{name:'File'})).toBeTruthy();
  fireEvent.click(screen.getByRole('tab',{name:'Effects'}));const value=screen.getByRole('spinbutton',{name:'Gain value'});fireEvent.change(value,{target:{value:'5'}});fireEvent.keyDown(value,{key:'Enter'});await settle();
  expect(writes(bridge,'transaction.apply')[0].ops).toEqual([{op:'set_clip',timeline:'t1',id:'c1',effects:[{on:true,id:undefined,effect:{kind:'gain',gain_db:5}}]}]);
});

it('uses the actual Fairlight inspector frame for track FX lane playback',async()=>{
  const t=song();(t.tracks[0] as any).lanes={'insert.fx.input_db':{points:[{at:{ticks:0,scale:48000},value:0},{at:{ticks:48000,scale:48000},value:10}]}};
  render(<FairlightInspector bridge={wire()} session={1} revision={7} timeline={t} media={[]} clips={[]} track="a" frame={t.rate.num/t.rate.den/2} onEdit={()=>{}}/>);
  fireEvent.click(screen.getByRole('tab',{name:'Effects'}));expect(screen.getByRole('slider',{name:'Input Level'}).getAttribute('aria-valuenow')).toBe('5');
});

it('keeps used VCA strips in shared layout order without changing audio routing or history',()=>{
  const t=song();(t.tracks[0] as any).strip.vcas=['V1'];t.vcas=[{id:'V1',name:'Master',fader_db:2},{id:'V2',name:'Unused'}];const bridge=wire();
  function Host(){const [layout,setLayout]=useMixerLayout(bridge,1,t.id);return <><IndexTracks timeline={t} layout={layout} hidden={[]} onHidden={()=>{}} onSelect={()=>{}} onEdit={()=>{throw Error('view move wrote project');}} onReorder={(keys,before)=>setLayout({order:moveMixerItems(mixerOrder(t,layout),keys,before)})}/><Mixer bridge={bridge} session={1} revision={7} timeline={t} frame={0} meters={null} selected={[]} onSelect={()=>{}}/></>;}
  const view=render(<Host/>);const data:Record<string,string>={};const transfer={types:['application/x-columbina-mixer-order'],setData:(k:string,v:string)=>{data[k]=v;},getData:(k:string)=>data[k]??''};
  fireEvent.dragStart(view.container.querySelector('[data-index-owner="vca:V1"]')!,{dataTransfer:transfer});fireEvent.drop(view.container.querySelector('[data-index-track="a"]')!,{dataTransfer:transfer});
  const strips=[...view.container.querySelectorAll('[data-mixer-strip]')].map(n=>n.getAttribute('data-mixer-strip'));
  expect(strips).toEqual(['vca:V1','track:a']);expect(writes(bridge,'transaction.apply')).toEqual([]);
  expect(screen.getByRole('slider',{name:'Fader Master'})).toBeTruthy();expect(screen.queryByRole('slider',{name:'Fader Unused'})).toBeNull();
});

it('keeps a held Provider field active after another field wheel expires',async()=>{
  vi.useFakeTimers();const t=song(),bridge=wire([...parameters,parameter('insert.fx.ceiling_db',-1)]);
  render(<AutomationTouchProvider bridge={bridge} session={1} revision={7} timeline={t} enabled recording playing show={()=>{}} onFailure={()=>{}}><EffectPanel bridge={bridge} session={1} revision={7} timeline={t} target={{track:'a'}} index={0}/></AutomationTouchProvider>);await settle();
  fireEvent.wheel(screen.getByRole('slider',{name:'Input Level'}),{deltaY:-1});const cap=screen.getByRole('slider',{name:'Ceiling'});fireEvent.pointerDown(cap,{button:0,clientY:100});
  await act(async()=>vi.advanceTimersByTimeAsync(500));fireEvent.pointerMove(cap,{clientY:120});await act(async()=>vi.advanceTimersByTimeAsync(10));
  expect(writes(bridge).at(-1)).toMatchObject({path:'insert.fx.ceiling_db',value:-3,touching:true});fireEvent.pointerUp(cap);await settle();expect(writes(bridge).at(-1)).toMatchObject({path:'insert.fx.ceiling_db',value:-3,touching:false});
});
