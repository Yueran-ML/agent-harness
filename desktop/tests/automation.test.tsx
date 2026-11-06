// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { TouchQueue, AutomationTouchProvider, useAutomationTouch } from '../src/studio/fairlight/AutomationTouch';
import { AutomationToolbar } from '../src/studio/fairlight/AutomationToolbar';
import { defaultFairlightView } from '../src/studio/fairlight/viewState';
import { laneValue, pixelAtSample, positionValue, sampleAtPixel, Pencil, defaultRecord } from '../src/studio/fairlight/automation';
import { AutomationLane } from '../src/studio/fairlight/AutomationLanes';
import { Mixer } from '../src/studio/fairlight/Mixer';
import { EqPanel } from '../src/studio/audio/EqPanel';
import { DynamicsPanel } from '../src/studio/audio/DynamicsPanel';
import { defaultEq, defaultDynamics } from '../src/studio/audio/processorModel';
import { timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';
import type { AutomationTouch, EngineAnswer, Timeline } from '../src/studio/engine';
it('keeps interleaved fader wheels owned and ignores idle Path Settings teardown',async()=>{
 vi.useFakeTimers();
 const t={...timeline([]),tracks:['a1','a2'].map((id,i)=>({id,name:`Voice ${i+1}`,kind:'audio',enabled:true,locked:false,clips:[],transitions:[],automation_arm:true,strip:{fader_db:i?-10:0}})),audio:{record:{...defaultRecord(),touch:'latch'}}} as unknown as Timeline;
 const call=vi.fn(async(method:string,params:any)=>({ok:true as const,result:{revision:8,done:[]}}));
 render(<AutomationTouchProvider bridge={{call}} session={1} timeline={t} enabled recording show={()=>{}} onFailure={e=>{throw e;}}><Mixer bridge={{call}} session={1} revision={7} timeline={t} frame={0} meters={null} selected={[]} onSelect={()=>{}}/></AutomationTouchProvider>);
 const a=screen.getByRole('slider',{name:'Fader Voice 1'}),b=screen.getByRole('slider',{name:'Fader Voice 2'});
 const touches=()=>call.mock.calls.filter(c=>c[0]==='automation.touch').map(c=>c[1]);
 fireEvent.wheel(a,{deltaY:-1});await act(async()=>vi.advanceTimersByTimeAsync(100));
 fireEvent.wheel(b,{deltaY:-1});await act(async()=>vi.advanceTimersByTimeAsync(100));
 fireEvent.wheel(a,{deltaY:-1});await settle();
 expect(touches().filter(m=>m.track==='a2'&&m.touching).every(m=>m.value<0)).toBe(true);
 expect(touches().filter(m=>m.track==='a1'&&m.touching).at(-1).value).toBeGreaterThan(0);
 await act(async()=>vi.advanceTimersByTimeAsync(400));
 expect(touches().at(-1)).toMatchObject({track:'a1',value:1,touching:true});
 await act(async()=>vi.advanceTimersByTimeAsync(100));expect(touches().at(-1)).toMatchObject({track:'a1',touching:false});
 fireEvent.click(screen.getByRole('button',{name:'Path Settings Voice 1'}));
 fireEvent.wheel(b,{deltaY:-1});await act(async()=>vi.advanceTimersByTimeAsync(10));
 fireEvent.click(screen.getByRole('button',{name:'Close processor'}));await settle();
 expect(touches().at(-1)).toMatchObject({track:'a2',touching:true});
 await act(async()=>vi.advanceTimersByTimeAsync(500));expect(touches().at(-1)).toMatchObject({track:'a2',touching:false});
 expect(call.mock.calls.some(c=>c[0]==='transaction.apply')).toBe(false);
});
it('reads held Polarity so stopped Preview can toggle on and back off',async()=>{
 const t={...timeline([]),tracks:[{id:'a1',name:'Voice',kind:'audio',enabled:true,locked:false,clips:[],transitions:[],automation_arm:true}],audio:{record:{...defaultRecord(),preview:true}}} as unknown as Timeline;
 const call=vi.fn(async(method:string,params:any)=>({ok:true as const,result:{revision:8,done:[]}}));
 render(<AutomationTouchProvider bridge={{call}} session={1} timeline={t} enabled recording playing={false} show={()=>{}} onFailure={e=>{throw e;}}><Mixer bridge={{call}} session={1} revision={7} timeline={t} frame={0} meters={null} selected={[]} onSelect={()=>{}}/></AutomationTouchProvider>);
 fireEvent.click(screen.getByRole('button',{name:'Path Settings Voice'}));const polarity=screen.getByRole('checkbox',{name:'Polarity'}) as HTMLInputElement;
 fireEvent.click(polarity);await settle();expect(polarity.checked).toBe(true);
 fireEvent.click(polarity);await settle();expect(polarity.checked).toBe(false);
 expect(call.mock.calls.filter(c=>c[0]==='automation.touch'&&!c[1].touching).map(c=>c[1].value)).toEqual([1,0]);
 expect(call.mock.calls.some(c=>c[0]==='transaction.apply')).toBe(false);
});
it('rejects stopped non-Preview touches without held values carrying into playback',async()=>{
 const t={...timeline([]),tracks:[{id:'a1',kind:'audio',name:'Voice',enabled:true,locked:false,clips:[],transitions:[],automation_arm:true}],audio:{record:{...defaultRecord(),touch:'latch'}}} as unknown as Timeline;
 const call=vi.fn(async()=>({ok:true as const,result:{}})),target={of:{track:'a1'},path:'fader_db'};
 function Control(){const a=useAutomationTouch()!;return <><button onClick={()=>{a.start(target,0);a.change(5);a.end();}}>Touch stopped</button><output data-state={a.status(target)}>{a.reading(target,0)}</output></>;}
 const props={bridge:{call},session:1,timeline:t,enabled:true,recording:true,show:()=>{},onFailure:(e:unknown)=>{throw e;}};
 const s=render(<AutomationTouchProvider {...props} playing={false}><Control/></AutomationTouchProvider>);
 fireEvent.click(screen.getByText('Touch stopped'));await settle();expect(screen.getByRole('status').textContent).toBe('0');expect(screen.getByRole('status').hasAttribute('data-state')).toBe(false);expect(call).not.toHaveBeenCalled();
 s.rerender(<AutomationTouchProvider {...props} playing><Control/></AutomationTouchProvider>);expect(screen.getByRole('status').textContent).toBe('0');
});
it('uses restored Active Range endpoints after undo for a single mark and Enable',()=>{
 const point=(seconds:number)=>({ticks:seconds*48000,scale:48000});
 const base={...timeline([]),audio:{record:{...defaultRecord(),active_range:{start:point(0),end:point(10),on:false}}}} as Timeline;
 const call=vi.fn(async()=>({ok:true as const,result:{}})),edit=vi.fn();
 const props={bridge:{call},session:1,timeline:base,view:{...defaultFairlightView(),automation:true,automationControls:true},update:vi.fn(),recording:false,selected:[],frame:20*base.rate.num/base.rate.den,onEdit:edit,onFailure:vi.fn(),onToggle:vi.fn(),marks:()=>({in:null,out:null})};
 const s=render(<AutomationToolbar {...props}/>);fireEvent.click(screen.getByText('Mark Active Out'));
 const changed={...base,audio:{record:edit.mock.calls.at(-1)![0][0].patch.record}} as Timeline;
 s.rerender(<AutomationToolbar {...props} timeline={changed}/>);
 s.rerender(<AutomationToolbar {...props} frame={5*base.rate.num/base.rate.den}/>);
 fireEvent.click(screen.getByText('Mark Active In'));expect(edit.mock.calls.at(-1)![0][0].patch.record.active_range).toEqual({start:point(5),end:point(10),on:false});
 s.rerender(<AutomationToolbar {...props} timeline={{...base,audio:{record:edit.mock.calls.at(-1)![0][0].patch.record}} as Timeline}/>);
 s.rerender(<AutomationToolbar {...props}/>);fireEvent.click(screen.getByLabelText('Active Range'));
 expect(edit.mock.calls.at(-1)![0][0].patch.record.active_range).toEqual({start:point(0),end:point(10),on:true});
});
const settle=()=>act(async()=>{for(let i=0;i<15;i++)await Promise.resolve();});
beforeEach(()=>{likeABrowser();vi.stubGlobal('PointerEvent',MouseEvent);});
afterEach(()=>{cleanup();vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();});
it('uses timeline samples, model linear dB/log/switch values and both pencil directions',()=>{
 const t=timeline([]);t.rate={num:30000,den:1001};
 expect(sampleAtPixel(pixelAtSample(12345,4,t),4,t)).toBe(12345);
 const points=[{at:{ticks:0,scale:48000},value:-20},{at:{ticks:48000,scale:48000},value:0}];
 expect(laneValue({points},.5,99)).toBe(-10);expect(laneValue({points},2,99)).toBe(0);
 expect(laneValue({kind:'switch',points:[{...points[0],value:0},{...points[1],value:1}]},.5,99)).toBe(0);
 expect(laneValue({scale:'log',points:[{...points[0],value:100},{...points[1],value:10000}]},.5,99)).toBeCloseTo(1000);
 expect(positionValue(.51,{path:'mute',kind:'switch',scale:'linear',min:0,max:1,value:0})).toBe(1);
 const pencil=new Pencil(48000);pencil.add(0,0,0);pencil.add(480,1,5);pencil.add(960,2,10);pencil.add(480,3,20);
 expect(pencil.finish().map(p=>[p.at.ticks,p.value])).toEqual([[0,0],[480,3],[960,2]]);
});
it('coalesces delayed touch sends and always releases the latest value, including teardown',async()=>{
 vi.useFakeTimers();const sent:AutomationTouch[]=[];let resolve!:()=>void;
 const queue=new TouchQueue('t1',async message=>{sent.push(message);if(sent.length===1)await new Promise<void>(r=>{resolve=r;});},e=>{throw e;});
 queue.start({of:{track:'a1'},path:'fader_db'},0);queue.change(1);await vi.advanceTimersByTimeAsync(10);queue.change(2);await vi.advanceTimersByTimeAsync(10);queue.change(3);queue.end();resolve();await queue.settled();
 expect(sent.map(m=>[m.value,m.touching])).toEqual([[0,true],[3,true],[3,false]]);
});
it('lane keyboard cut cannot escape into clip edits',async()=>{
 const t=timeline([]), call=vi.fn(async(method:string):Promise<EngineAnswer>=>({ok:true,result:method==='automation.parameters'?{revision:7,of:{track:'a1'},parameters:[{path:'fader_db',kind:'continuous',scale:'linear',value:0,min:-100,max:10}]}:{revision:8,done:[]}}));
 const bubbled=vi.fn();render(<div onKeyDown={bubbled}><AutomationLane bridge={{call} as never} session={1} revision={7} timeline={t} of={{track:'a1'}} lanes={{fader_db:{points:[{at:{ticks:0,scale:48000},value:-10},{at:{ticks:48000,scale:48000},value:0}]}}} scale={4} from={0} to={100} height={128} frame={0} snapping={false} marks={()=>({in:null,out:null})} path="fader_db" pencil={false} onPath={()=>{}} onEdit={()=>{}} name="Voice"/></div>);await settle();
 expect((screen.getByRole('option',{name:'Fader'}) as HTMLOptionElement).value).toBe('fader_db');
 const svg=screen.getByLabelText('Voice Fader');fireEvent.keyDown(svg,{key:'a',ctrlKey:true});fireEvent.keyDown(svg,{key:'x',ctrlKey:true});await settle();
 expect(bubbled).not.toHaveBeenCalled();expect(call.mock.calls.filter(c=>c[0]==='transaction.apply')).toHaveLength(1);
 const ops=(call.mock.calls as any[]).find(c=>c[0]==='transaction.apply')[1].ops;
 expect(ops[0]).toMatchObject({edit:'lane_points',action:'remove',of:{track:'a1'},path:'fader_db'});
});
it('recorded fader and mute issue touch RPCs; the same Edit mixer remains read-only on existing lanes',async()=>{
 const t:Timeline={...timeline([]),tracks:[{id:'a1',name:'Voice',kind:'audio',enabled:true,locked:false,clips:[],transitions:[],automation_arm:true,lanes:{fader_db:{points:[{at:{ticks:0,scale:48000},value:-12}]}},strip:{fader_db:0} as never}],audio:{record:defaultRecord()} as never};
 const call=vi.fn(async()=>({ok:true as const,result:{revision:8,done:[]}}));
 const props={bridge:{call},session:1,revision:7,timeline:t,frame:0,meters:null,selected:[],onSelect:()=>{}};
 const rendered=render(<AutomationTouchProvider bridge={{call}} session={1} timeline={t} enabled recording show={()=>{}} onFailure={e=>{throw e;}}><Mixer {...props}/></AutomationTouchProvider>);
 const cap=screen.getByRole('slider',{name:'Fader Voice'});fireEvent.pointerDown(cap,{button:0,pointerId:1,clientY:100});fireEvent.pointerMove(cap,{pointerId:1,clientY:80});fireEvent.pointerUp(cap,{pointerId:1,clientY:80});await settle();
 const messages=(call.mock.calls as any[]).filter(c=>c[0]==='automation.touch').map(c=>c[1]);expect(messages[0]).toMatchObject({path:'fader_db',touching:true,value:-12});expect(messages.at(-1).touching).toBe(false);expect(call.mock.calls.some(c=>(c as any[])[0]==='transaction.apply')).toBe(false);
 fireEvent.pointerDown(screen.getByRole('button',{name:/mute Voice/i}),{button:0,pointerId:2});await settle();expect((call.mock.calls as any[]).at(-1)[1]).toMatchObject({path:'mute',value:1,touching:false});
 rendered.unmount();call.mockClear();render(<Mixer {...props}/>);expect(screen.getByRole('slider',{name:'Fader Voice'}).getAttribute('aria-readonly')).toBe('true');
});
function recordingProcessor(module:'eq'|'dynamics', categories={}) {
 const t:Timeline={...timeline([]),tracks:[{id:'a1',name:'Voice',kind:'audio',enabled:true,locked:false,clips:[],transitions:[],automation_arm:true,strip:{eq:defaultEq(),dynamics:defaultDynamics()} as never}],audio:{record:{...defaultRecord(),categories:{...defaultRecord().categories,...categories}}} as never};
 const parameters=[['eq.1.freq',100],['eq.1.gain_db',0],['eq.1.q',1],['eq.on',0],['eq.gain_db',0],['dynamics.compressor.ratio',2],['dynamics.expander.on',0],['dynamics.makeup_db',0]].map(([path,value])=>({path,value,kind:'continuous',scale:'linear',min:0,max:20000}));
 const call=vi.fn(async(method:string,params:any):Promise<EngineAnswer>=>({ok:true,result:method==='automation.parameters'?{revision:7,of:{track:'a1'},parameters}:method==='audio.eq_response'?{revision:7,sample_rate:48000,frequencies:params.frequencies,gain_db:params.frequencies.map(()=>0)}:method==='audio.dynamics_curve'?{revision:7,inputs_db:params.inputs_db,outputs_db:params.inputs_db}:{revision:7,done:[]}}));
 const props={bridge:{call},session:1,revision:7,timeline:t,target:{track:'a1'}};
 const rendered=render(<AutomationTouchProvider bridge={{call}} session={1} revision={7} timeline={t} enabled recording show={()=>{}} onFailure={e=>{throw e;}}>{module==='eq'?<EqPanel {...props}/>:<DynamicsPanel {...props}/>}</AutomationTouchProvider>);
 return {call,...rendered,touches:()=>call.mock.calls.filter(c=>c[0]==='automation.touch').map(c=>c[1])};
}
it('records both EQ handle axes, switches Q wheel to the two-axis gesture, and releases every path without static patches',async()=>{
 const s=recordingProcessor('eq');await settle();const handle=screen.getByRole('slider',{name:'EQ band 2'});
 fireEvent.wheel(handle,{deltaY:-1});await settle();
 fireEvent.pointerDown(handle,{button:0,clientX:100,clientY:80});
 fireEvent.pointerMove(handle,{clientX:130,clientY:60});fireEvent.pointerUp(handle);await settle();
 for(const path of ['eq.1.q','eq.1.freq','eq.1.gain_db']){
   const values=s.touches().filter(m=>m.path===path);expect(values[0].touching).toBe(true);expect(values.at(-1).touching).toBe(false);
 }
 expect(s.touches().filter(m=>m.path==='eq.1.freq').at(-1).value).toBeGreaterThan(100);
 expect(s.touches().filter(m=>m.path==='eq.1.gain_db').at(-1).value).toBeGreaterThan(0);
 fireEvent.click(screen.getByRole('checkbox',{name:'Enable'}));await settle();expect(s.touches().at(-1)).toMatchObject({path:'eq.on',value:1,touching:false});
 expect(s.call.mock.calls.some(c=>c[0]==='transaction.apply')).toBe(false);
});
it('records processor numeric entry and teardown release; disabled recording categories never fall through to static edits',async()=>{
 const s=recordingProcessor('dynamics',{gate:false});await settle();
 const ratio=screen.getByRole('spinbutton',{name:'Compressor Ratio value'});fireEvent.change(ratio,{target:{value:'3.7'}});fireEvent.keyDown(ratio,{key:'Enter'});await settle();expect(s.touches().at(-1)).toMatchObject({path:'dynamics.compressor.ratio',value:3.7,touching:false});
 fireEvent.click(screen.getByRole('checkbox',{name:'Expander'}));await settle();expect(s.touches().some(m=>m.path.startsWith('dynamics.expander'))).toBe(false);
 const knob=screen.getByRole('slider',{name:'Make Up'});fireEvent.pointerDown(knob,{button:0,clientY:80});fireEvent.pointerMove(knob,{clientY:60});s.unmount();await settle();
 expect(s.touches().at(-1)).toMatchObject({path:'dynamics.makeup_db',value:2,touching:false});expect(s.call.mock.calls.some(c=>c[0]==='transaction.apply')).toBe(false);
});
it('holds Preview between stops and returns a released Trim fader to unity under Snap',async()=>{
 vi.useFakeTimers();localStorage.setItem('fairlight.glide_ms','250');
 const t={...timeline([]),tracks:[{id:'a1',kind:'audio',name:'Voice',enabled:true,locked:false,clips:[],transitions:[],automation_arm:true}],audio:{record:{...defaultRecord(),touch:'snap',mode:'trim'}}} as unknown as Timeline;
 const call=vi.fn(async()=>({ok:true as const,result:{}})),target={of:{track:'a1'},path:'fader_db'};
 function Control(){const a=useAutomationTouch()!;return <><button onClick={()=>{a.start(target,0);a.change(5);a.end();}}>Touch</button><output>{a.reading(target,0).toFixed(1)}</output></>;}
 const props={bridge:{call},session:1,timeline:t,enabled:true,recording:true,show:()=>{},onFailure:(e:unknown)=>{throw e;}};
 const s=render(<AutomationTouchProvider {...props}><Control/></AutomationTouchProvider>);
 fireEvent.click(screen.getByText('Touch'));expect(screen.getByRole('status').textContent).toBe('5.0');
 await act(async()=>vi.advanceTimersByTimeAsync(300));expect(screen.getByRole('status').textContent).toBe('0.0');
 const preview={...t,audio:{...t.audio!,record:{...t.audio!.record,preview:true}}};
 s.rerender(<AutomationTouchProvider {...props} timeline={preview}><Control/></AutomationTouchProvider>);fireEvent.click(screen.getByText('Touch'));
 s.rerender(<AutomationTouchProvider {...props} timeline={preview} playing={false}><Control/></AutomationTouchProvider>);await act(async()=>vi.advanceTimersByTimeAsync(500));expect(screen.getByRole('status').textContent).toBe('5.0');
 s.rerender(<AutomationTouchProvider {...props} timeline={preview} read><Control/></AutomationTouchProvider>);expect(screen.getByRole('status').textContent).toBe('0.0');
});
it.each([true, false])('does not continuously redraw held Preview/Latch values (preview=%s)', async preview => {
 vi.useFakeTimers();
 const t={...timeline([]),tracks:[{id:'a1',kind:'audio',name:'Voice',enabled:true,locked:false,clips:[],transitions:[],automation_arm:true}],audio:{record:{...defaultRecord(),touch:'latch',preview}}} as unknown as Timeline;
 const call=vi.fn(async()=>({ok:true as const,result:{}})),target={of:{track:'a1'},path:'fader_db'};
 let renders=0;
 function Control(){renders++;const a=useAutomationTouch()!;return <><button onClick={()=>{a.start(target,0);a.change(5);a.end();}}>Hold</button><output>{a.reading(target,0)}</output></>;}
 render(<AutomationTouchProvider bridge={{call}} session={1} timeline={t} enabled recording show={()=>{}} onFailure={e=>{throw e;}}><Control/></AutomationTouchProvider>);
 fireEvent.click(screen.getByText('Hold'));await settle();
 const held=renders;
 await act(async()=>vi.advanceTimersByTimeAsync(500));
 expect(screen.getByRole('status').textContent).toBe('5');
 expect(renders).toBe(held);
 expect(vi.getTimerCount()).toBe(0);
});
it('toolbar keeps Active Range marks separate, remaps copied automation to the selected destination, and dispatches Follows Edit',async()=>{
 const t={...timeline([]),audio:{record:defaultRecord()}} as Timeline;
 const call=vi.fn(async()=>({ok:true as const,result:{lanes:[{of:{track:'a1'},path:'fader_db',value:-12}]}}));const edit=vi.fn(),fail=vi.fn();
 const props={bridge:{call},session:1,timeline:t,view:{...defaultFairlightView(),automation:true,automationControls:true},update:vi.fn(),recording:false,selected:['track:a1'],frame:10,onEdit:edit,onFailure:fail,onToggle:vi.fn(),marks:()=>({in:null,out:null})};
 const s=render(<AutomationToolbar {...props}/>);fireEvent.click(screen.getByText('Mark Active In'));
 s.rerender(<AutomationToolbar {...props} frame={20}/>);fireEvent.click(screen.getByText('Mark Active Out'));fireEvent.click(screen.getByLabelText('Active Range'));
 expect(edit.mock.calls.at(-1)?.[0][0].patch.record.active_range).toMatchObject({on:true,start:sampleAtTime(10),end:sampleAtTime(20)});
 fireEvent.click(screen.getByText('Copy automation'));await settle();s.rerender(<AutomationToolbar {...props} selected={['track:a2']}/>);fireEvent.click(screen.getByText('Paste automation'));
 expect(edit.mock.calls.at(-1)?.[0][0].lanes).toEqual([{of:{track:'a2'},path:'fader_db',value:-12}]);
 fireEvent.click(screen.getByLabelText('Automation Follows Edit'));expect(edit.mock.calls.at(-1)?.[0][0]).toMatchObject({op:'set_timeline_audio',patch:{follows_edit:false}});expect(fail).not.toHaveBeenCalled();
 function sampleAtTime(frame:number){return {ticks:Math.round(frame*t.rate.den/t.rate.num*t.sample_rate),scale:t.sample_rate};}
});
it('Pencil captures the full lane and delegates sampled points to lane_draw; shifted nudge uses the physical comma key',async()=>{
 let revision=7;
 const t=timeline([]),call=vi.fn(async(method:string):Promise<EngineAnswer>=>({ok:true,result:method==='automation.parameters'?{revision,of:{track:'a1'},parameters:[{path:'fader_db',kind:'continuous',scale:'linear',value:0,min:-100,max:10}]}:{revision:revision+1,done:[]}}));
 const props={bridge:{call} as never,session:1,revision:7,timeline:t,of:{track:'a1'},lanes:{fader_db:{points:[{at:{ticks:48000,scale:48000},value:0}]}},scale:4,from:0,to:100,height:128,frame:0,snapping:false,marks:()=>({in:null,out:null}),path:'fader_db',pencil:true,onPath:()=>{},onEdit:()=>{},name:'Voice'};
 const s=render(<AutomationLane {...props}/>);await settle();
 const svg=screen.getByLabelText('Voice Fader'),area=s.container.querySelector('svg rect')!;expect(area.getAttribute('height')).toBe('128');
 fireEvent.pointerDown(area,{button:0,clientX:0,clientY:110});fireEvent.pointerMove(svg,{clientX:40,clientY:100});fireEvent.pointerUp(svg,{clientX:80,clientY:90});await settle();
 const writes=()=>call.mock.calls.filter(c=>c[0]==='transaction.apply') as any[];
 expect(writes()[0][1].ops[0]).toMatchObject({edit:'lane_draw',of:{track:'a1'},path:'fader_db'});expect(writes()[0][1].ops[0].points.length).toBeGreaterThanOrEqual(2);
 revision=8;s.rerender(<AutomationLane {...props} revision={revision}/>);await settle();
 const next=screen.getByLabelText('Voice Fader');fireEvent.keyDown(next,{key:'a',ctrlKey:true});fireEvent.keyDown(next,{key:'<',code:'Comma',shiftKey:true});await settle();expect(writes().at(-1)[1].ops[0].by.ticks).toBe(-Math.round(t.sample_rate*t.rate.den/t.rate.num)*5);
});
