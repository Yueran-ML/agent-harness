// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { FxMeter } from '../src/studio/audio/FxMeter';
import { NoiseLearn } from '../src/studio/audio/NoiseLearn';
import { useState } from 'react';
import { IndexTracks } from '../src/studio/fairlight/IndexTracks';
import { Mixer } from '../src/studio/fairlight/Mixer';
import { mixerOrder, moveMixerItems, defaultMixerLayout, resetBusOrder } from '../src/studio/audio/mixerLayout';
import { likeABrowser } from './mounted';
import { timeline } from './timeline-fixture';
beforeEach(()=>{likeABrowser();vi.stubGlobal('PointerEvent',MouseEvent);});
afterEach(()=>{cleanup();vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();});
const settle=()=>act(async()=>{for(let i=0;i<12;i++)await Promise.resolve();});
const model=()=>({...timeline([]),tracks:[{id:'a',kind:'audio' as const,name:'Audio',enabled:true,locked:false,clips:[],transitions:[]}],buses:[{id:'b',name:'Bus',color:'#777777ff',format:'stereo' as const}]});
it('shares one visible meter poll, shows only matching DSP IDs, and clears stopped graphs',async()=>{
  vi.useFakeTimers();let playing=true;
  const call=vi.fn(async()=>({ok:true as const,result:{timeline:'t1',playing,generation:1,sequence:1,sample_rate:48000,readings:[{owner:{track:'a'},insert:'fx1',slot:0,reading:{kind:'limiter',input_db:-12,reduction_db:-3,output_db:-15}}]}}));
  const props={bridge:{call} as any,session:1,revision:1,timeline:model(),target:{track:'a'}};
  const view=render(<><FxMeter context={props} kind="limiter" insert="fx1"/><FxMeter context={props} kind="limiter" insert="missing"/></>);await settle();
  expect(call).toHaveBeenCalledTimes(1);expect(screen.getByText('-12.0 dB')).toBeTruthy();expect(screen.getAllByText('No live audio')).toHaveLength(1);
  playing=false;await act(async()=>{vi.advanceTimersByTime(100);});await settle();expect(screen.queryByText('-12.0 dB')).toBeNull();
  view.unmount();const requests=call.mock.calls.length;await act(async()=>{vi.advanceTimersByTime(1000);});expect(call).toHaveBeenCalledTimes(requests);
});
it('discards meter replies from an old session and does not invent a dynamics ball',async()=>{
  const pending:((x:any)=>void)[]=[];const call=vi.fn(()=>new Promise<any>(resolve=>pending.push(resolve)));
  const props={bridge:{call} as any,session:1,revision:1,timeline:model(),target:{track:'a'}};
  const view=render(<FxMeter context={props} kind="dynamics"/>);view.rerender(<FxMeter context={{...props,session:2}} kind="dynamics"/>);
  pending[0]({ok:true,result:{timeline:'t1',playing:true,generation:1,readings:[{owner:{track:'a'},insert:null,reading:{kind:'dynamics',input_db:-2,output_db:-10}}]}});await settle();
  expect(screen.queryByLabelText('Live dynamics ball')).toBeNull();expect(screen.getByText('No live audio')).toBeTruthy();
});
it('learns with owner/slot/range/revision, accepts an early completion, and cancels its pending job on unmount',async()=>{
  let listener:(event:any)=>void=()=>{};let reply:(value:any)=>void=()=>{};let early=true;
  const call=vi.fn((method:string,params:any)=>{if(method==='audio.noise_learn'){if(early){listener({kind:'notification',session:1,method:'audio.analysis',params:{job:'j1',state:'done',method:'audio.noise_learn',result:{applied:{revision:2}}}});return Promise.resolve({ok:true,result:{job:'j1'}});}return new Promise(resolve=>{reply=resolve;});}return Promise.resolve({ok:true,result:{}});});
  const props={bridge:{call,onEvent:(fn:any)=>{listener=fn;return()=>{};}} as any,session:1,revision:7,timeline:model(),target:{track:'a'}};
  const view=render(<NoiseLearn context={props} slot={2} insert="fx3" disabled={false}/>);
  fireEvent.click(screen.getByRole('button',{name:'Learn Noise'}));await settle();expect(call).toHaveBeenCalledWith('audio.noise_learn',{timeline:'t1',target:{track:'a'},slot:2,range:{from:0,to:25},revision:7},{expectedSession:1});expect(screen.getByText('Noise learned')).toBeTruthy();
  early=false;fireEvent.click(screen.getByRole('button',{name:'Learn Noise'}));view.unmount();reply({ok:true,result:{job:'j2'}});await settle();expect(call).toHaveBeenCalledWith('audio.cancel',{job:'j2'},{expectedSession:1});
});
it('shows stale Learn refusal and never submits a static replacement',async()=>{
  let listener:(event:any)=>void=()=>{};const call=vi.fn(async()=>({ok:true,result:{job:'j'}}));
  render(<NoiseLearn context={{bridge:{call,onEvent:(fn:any)=>{listener=fn;return()=>{};}} as any,session:1,revision:1,timeline:model(),target:{track:'a'}}} slot={0} disabled={false}/>);
  fireEvent.click(screen.getByRole('button',{name:'Learn Noise'}));await settle();
  act(()=>listener({kind:'notification',session:1,method:'audio.analysis',params:{job:'j',state:'done',method:'audio.noise_learn',result:{applied:null,refused:{message:'stale revision'}}}}));
  expect(screen.getByRole('alert').textContent).toBe('stale revision');expect(call).toHaveBeenCalledTimes(1);
});
it('keeps Mixer menu changes in shared view state and box selection out of project history',()=>{
  const call=vi.fn(),select=vi.fn();const view=render(<Mixer bridge={{call} as any} session={1} revision={1} timeline={model()} frame={0} meters={null} selected={[]} onSelect={select}/>);
  fireEvent.click(screen.getByRole('button',{name:'Mixer options'}));fireEvent.click(screen.getByRole('menuitemradio',{name:'Small Track Mixer'}));expect(view.container.querySelector('.audio-mixer')?.getAttribute('data-small')).toBe('true');
  fireEvent.click(screen.getByRole('checkbox',{name:'Single Mixer'}));expect(view.container.querySelectorAll('.mixer-region')).toHaveLength(1);
  const strip=view.container.querySelector('[data-mixer-strip="track:a"]')!;vi.spyOn(strip,'getBoundingClientRect').mockReturnValue({left:10,right:80,top:10,bottom:100,width:70,height:90,x:10,y:10,toJSON:()=>({})});
  const region=view.container.querySelector('.mixer-region')!;fireEvent.pointerDown(region,{button:0,clientX:0,clientY:0});fireEvent.pointerMove(region,{clientX:90,clientY:110});fireEvent.pointerUp(region);
  expect(select).toHaveBeenLastCalledWith(expect.arrayContaining(['track:a']));expect(call).not.toHaveBeenCalled();
});
it('shares the divider order across multi-drag and reset without changing timeline track order',()=>{
  const t=model(),layout=defaultMixerLayout();let order=mixerOrder(t,layout);expect(order).toEqual(['track:a','divider','bus:b']);
  order=moveMixerItems(order,['bus:b'],'track:a');expect(order).toEqual(['bus:b','track:a','divider']);
  expect(resetBusOrder(t,{...layout,order})).toEqual(['track:a','divider','bus:b']);expect(t.tracks[0].id).toBe('a');
});

it('keeps Index audio drag and arrows in project history while bus order and Undo share layout',()=>{
  const initial={...model(),tracks:[...timeline().tracks.filter(t=>t.kind!=='audio'),...['a','c','d'].map(id=>({...model().tracks[0],id,name:id}))]};
  const edits:any[][]=[];
  function Host(){const [t,setT]=useState(initial),[layout,setLayout]=useState(defaultMixerLayout());
    return <><button onClick={()=>setT(initial)}>Undo fixture</button><IndexTracks timeline={t} layout={layout} hidden={[]} onHidden={()=>{}} onSelect={()=>{}}
      onReorder={(keys,before)=>setLayout({...layout,order:moveMixerItems(mixerOrder(t,layout),keys,before)})}
      onEdit={ops=>{edits.push(ops);setT(old=>{const tracks=[...old.tracks];for(const op of ops)if('op' in op&&op.op==='move_track'){const at=tracks.findIndex(t=>t.id===op.id);tracks.splice(op.index,0,tracks.splice(at,1)[0]);}return {...old,tracks};});}}/>
      <output data-testid="project-order">{t.tracks.map(t=>t.id).join(',')}</output></>;
  }
  const view=render(<Host/>);const data:Record<string,string>={};const dataTransfer={types:['application/x-columbina-mixer-order'],setData:(k:string,v:string)=>{data[k]=v;},getData:(k:string)=>data[k]??''};
  const drag=(from:string,to:string)=>{fireEvent.dragStart(view.container.querySelector(from)!,{dataTransfer});fireEvent.drop(view.container.querySelector(to)!,{dataTransfer});};
  drag('[data-index-owner="bus:b"]','[data-index-track="a"]');expect(edits).toHaveLength(0);
  drag('[data-index-track="d"]','[data-index-track="a"]');expect(edits).toHaveLength(1);expect(edits[0]).toEqual([{op:'move_track',timeline:'t1',id:'d',index:initial.tracks.length-3}]);
  const rows=()=>[...view.container.querySelectorAll('[data-index-track]')].map(n=>n.getAttribute('data-index-track'));
  expect(rows().slice(-3)).toEqual(['d','a','c']);
  fireEvent.click(screen.getByRole('button',{name:'Move d down'}));expect(edits).toHaveLength(2);expect(rows().slice(-3)).toEqual(['a','d','c']);
  fireEvent.click(screen.getByRole('button',{name:'Undo fixture'}));expect(rows().slice(-3)).toEqual(['a','c','d']);
  const ownerRows=[...view.container.querySelectorAll('tbody tr')];expect(ownerRows.findIndex(n=>n.getAttribute('data-index-owner')==='bus:b')).toBeLessThan(ownerRows.findIndex(n=>n.getAttribute('data-index-track')==='a'));
});
