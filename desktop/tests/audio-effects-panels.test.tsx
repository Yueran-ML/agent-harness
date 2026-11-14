// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useState } from 'react';
import type { EngineBridge } from '../src/bridge';
import type { Effect, Insert, Timeline } from '../src/studio/engine';
import { DuckerPanel } from '../src/studio/audio/DuckerPanel';
import { EffectPanel } from '../src/studio/audio/EffectPanel';
import { InsertSlots } from '../src/studio/fairlight/InsertSlots';
import { defaultEffect } from '../src/studio/audio/effectModel';
import type { AudioAutomationAdapter } from '../src/studio/audio/automationAdapter';
import { likeABrowser } from './mounted';
import { timeline } from './timeline-fixture';
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
function setup(kind: Effect['kind'], options: { mono?: boolean; ducker?: boolean; slots?: boolean; count?: number; automation?: AudioAutomationAdapter } = {}) {
  let model: Timeline = { ...timeline([]), tracks: [{ id: 'a', kind: 'audio', name: 'Voice', enabled: true, locked: false, format: options.mono ? 'mono' : 'stereo', clips: [], transitions: [], strip: { inserts: Array.from({ length: options.count ?? 1 }, (_, i) => ({ id: `fx${i + 1}`, on: true, effect: defaultEffect(kind) })) } as any }] };
  let revision = 1; const requests: any[] = [];
  function Host() {
    const [state, setState] = useState({ timeline: model, revision });
    const bridge = { call: async (method: string, params: any) => {
      requests.push({ method, params }); if (method !== 'transaction.apply') throw Error(method);
      model = structuredClone(model); const strip = (model.tracks[0] as any).strip;
      for (const op of params.ops) {
        if (op.patch?.strip?.inserts) strip.inserts = op.patch.strip.inserts;
        if (op.edit === 'set_insert') { if (op.insert === null) strip.inserts.splice(op.slot, 1); else strip.inserts[op.slot] = { ...op.insert, id: 'fresh' }; }
      }
      revision++; setState({ timeline: model, revision }); return { ok: true, result: { revision, done: [] } };
    } } as Pick<EngineBridge, 'call'>;
    const props = { bridge, session: 1, ...state, target: { track: 'a' }, automation: options.automation };
    return options.ducker ? <DuckerPanel {...props} /> : options.slots ? <InsertSlots {...props} /> : <EffectPanel {...props} index={0} />;
  }
  return { ...render(<Host />), requests, insert: (): Insert => (model.tracks[0] as any).strip.inserts[0] };
}
it('drags with one cancellable transaction identity and keeps insert IDs', async () => {
  const s = setup('gain'); const knob = screen.getByRole('slider', { name: 'Gain' });
  fireEvent.pointerDown(knob, { button: 0, clientY: 100 }); fireEvent.pointerMove(knob, { clientY: 80 }); await settle();
  expect(s.insert()).toMatchObject({ id: 'fx1', effect: { gain_db: 2 } });
  fireEvent.keyDown(knob, { key: 'Escape' }); await settle(); expect(s.insert().effect).toMatchObject({ gain_db: 0 });
  expect(s.requests.every(r => r.params.ops.every((op: any) => op.edit === 'set_track_audio'))).toBe(true);
  expect(new Set(s.requests.map(r => r.params.gesture.id)).size).toBe(1);
});
it('exposes Voice restrictions, mono Width restriction, integer Pitch and all Limiter parameters', async () => {
  const nr = setup('noise_reduction'); fireEvent.change(screen.getByRole('combobox', { name: 'Mode' }), { target: { value: 'voice' } }); await settle();
  expect((screen.getByRole('spinbutton', { name: 'Threshold value' }) as HTMLInputElement).disabled).toBe(true);
  expect((screen.getByRole('spinbutton', { name: 'Dry/Wet value' }) as HTMLInputElement).disabled).toBe(false); nr.unmount();
  const width = setup('stereo_width', { mono: true }); expect((screen.getByRole('spinbutton', { name: 'Width value' }) as HTMLInputElement).disabled).toBe(true); width.unmount();
  const pitch = setup('pitch'); const semitone = screen.getByRole('spinbutton', { name: 'Semitones value' }); fireEvent.change(semitone, { target: { value: '3.6' } }); fireEvent.keyDown(semitone, { key: 'Enter' }); await settle(); expect(pitch.insert().effect).toMatchObject({ semitones: 4 }); pitch.unmount();
  setup('limiter'); expect(screen.getByRole('spinbutton', { name: 'Release value' })).toHaveProperty('min', '0.01'); expect(screen.getByRole('checkbox', { name: 'Sidechain' })).toBeTruthy(); expect(screen.getByRole('combobox', { name: 'Sidechain Source' })).toBeTruthy();
});
it('keeps full slots replaceable and exposes an actual parameter window', async () => {
  const s = setup('gain', { slots: true, count: 6 }); expect(screen.getByRole('button', { name: 'Add effect' })).toHaveProperty('disabled', true);
  fireEvent.change(screen.getByRole('combobox', { name: 'Replace effect 1' }), { target: { value: 'limiter' } }); await settle();
  expect(s.insert()).toMatchObject({ id: 'fresh', effect: { kind: 'limiter', ceiling_db: -1 } });
  fireEvent.click(screen.getByRole('button', { name: 'Limiter' })); expect(within(screen.getByRole('dialog', { name: 'Limiter' })).getByRole('spinbutton', { name: 'Ceiling value' })).toBeTruthy();
});
it('records exact insert path and never falls back to static writes when touch is refused', async () => {
  const automation: AudioAutomationAdapter = { enabled: true, recording: true, trim: false, preview: false, active: () => false, start: vi.fn(() => false), change: vi.fn(), end: vi.fn(), instant: vi.fn(() => false), show: vi.fn(), live: null };
  const s = setup('gain', { automation }); const knob = screen.getByRole('slider', { name: 'Gain' });
  fireEvent.pointerDown(knob, { button: 0, clientY: 100 }); fireEvent.pointerMove(knob, { clientY: 80 }); fireEvent.pointerUp(knob); await settle();
  expect(automation.start).toHaveBeenCalledWith({ of: { track: 'a' }, path: 'insert.fx1.gain_db' }, 0);
  const input = screen.getByRole('spinbutton', { name: 'Gain value' }); fireEvent.change(input, { target: { value: '-5' } }); fireEvent.keyDown(input, { key: 'Enter' }); await settle();
  expect(automation.instant).toHaveBeenCalledWith({ of: { track: 'a' }, path: 'insert.fx1.gain_db' }, -5); expect(s.requests).toEqual([]);
});
it('keeps FX numbers and switches absolute in Trim mode, which only offsets the fader',async()=>{
  const automation:AudioAutomationAdapter={enabled:true,recording:true,trim:true,preview:false,active:()=>false,start:vi.fn(()=>false),change:vi.fn(),end:vi.fn(),instant:vi.fn(()=>false),show:vi.fn(),live:null};
  setup('limiter',{automation});
  fireEvent.pointerDown(screen.getByRole('slider',{name:'Ceiling'}),{button:0,clientY:100});
  expect(automation.start).toHaveBeenCalledWith({of:{track:'a'},path:'insert.fx1.ceiling_db'},-1);
  fireEvent.click(screen.getByRole('checkbox',{name:'Enable'}));
  expect(automation.instant).toHaveBeenCalledWith({of:{track:'a'},path:'insert.fx1.on'},0);
});
it('Alt enable changes only matching selected insert switches, preserving each ID and settings', async () => {
  const tracks = ['a','b','c'].map((id,i)=>({id,kind:'audio' as const,name:id,enabled:true,locked:id==='c',clips:[],transitions:[],strip:{inserts:[{id:`${id}-fx`,on:true,effect:{kind:'gain',gain_db:i+2}}]}} as any));
  const call=vi.fn(async()=>({ok:true as const,result:{revision:2,done:[]}}));
  render(<EffectPanel bridge={{call} as any} session={1} revision={1} timeline={{...timeline([]),tracks}} target={{track:'a'}} selected={['track:a','track:b','track:c']} index={0}/>);
  fireEvent.click(screen.getByRole('checkbox',{name:'Enable'}),{altKey:true});await settle();
  const ops=(call.mock.calls as any)[0][1].ops;expect(ops).toHaveLength(2);
  expect(ops.map((op:any)=>op.patch.strip.inserts[0])).toEqual([{id:'a-fx',on:false,effect:{kind:'gain',gain_db:2}},{id:'b-fx',on:false,effect:{kind:'gain',gain_db:3}}]);
});
it('makes a copied effect immediately available to a different already-open panel',async()=>{
  const t={...timeline([]),tracks:['a','b'].map((id,i)=>({id,kind:'audio' as const,name:id,enabled:true,locked:false,clips:[],transitions:[],strip:{inserts:[{id:`${id}-fx`,on:true,effect:{kind:'gain',gain_db:i?0:7}}]}} as any))};
  const call=vi.fn(async()=>({ok:true as const,result:{revision:2,done:[]}}));const context={bridge:{call} as any,session:1,revision:1,timeline:t};
  render(<><EffectPanel {...context} target={{track:'a'}} index={0}/><EffectPanel {...context} target={{track:'b'}} index={0}/></>);
  const panels=screen.getAllByRole('region',{name:'Gain'});fireEvent.click(within(panels[0]).getByRole('button',{name:'Copy'}));
  const paste=within(panels[1]).getByRole('button',{name:'Paste'});expect(paste).toHaveProperty('disabled',false);fireEvent.click(paste);await settle();
  expect((call.mock.calls as any)[0][1].ops[0]).toMatchObject({track:'b',patch:{strip:{inserts:[{id:'b-fx',effect:{gain_db:7}}]}}});
});

it.each([
  { ducker: false, a: 'Input Level', b: 'Ceiling', pathA: 'insert.fx1.input_db', pathB: 'insert.fx1.ceiling_db' },
  { ducker: true, a: 'Duck Amount', b: 'Rise Time', pathA: 'track_fx.amount_db', pathB: 'track_fx.rise_ms' },
])('owns timers and cancellation per field in $ducker Ducker mode', async ({ducker,a,b,pathA,pathB}) => {
  vi.useFakeTimers(); let held: {of:any;path:string}|null=null; const currentPath = () => held?.path;
  const automation:AudioAutomationAdapter={enabled:true,recording:true,trim:false,preview:false,
    active: target=>Boolean(held && (!target || target.path===held.path)),
    start:vi.fn(target=>{held=target;return true;}), change:vi.fn(),end:vi.fn(()=>{held=null;}),instant:vi.fn(()=>true),show:vi.fn(),live:null};
  const s=setup('limiter',{automation,ducker});const A=screen.getByRole('slider',{name:a}),B=screen.getByRole('slider',{name:b});
  fireEvent.wheel(A,{deltaY:-1});expect(currentPath()).toBe(pathA);
  fireEvent.pointerDown(B,{button:0,clientY:100});expect(currentPath()).toBe(pathB);
  act(()=>vi.advanceTimersByTime(500));expect(currentPath()).toBe(pathB);
  fireEvent.keyDown(A,{key:'Escape'});expect(currentPath()).toBe(pathB);
  fireEvent.pointerMove(B,{clientY:80});expect(automation.change).toHaveBeenLastCalledWith(ducker?30:0);
  fireEvent.pointerUp(B);expect(held).toBeNull();
  fireEvent.wheel(A,{deltaY:-1});act(()=>vi.advanceTimersByTime(100));fireEvent.wheel(B,{deltaY:-1});
  act(()=>vi.advanceTimersByTime(100));fireEvent.wheel(A,{deltaY:-1});expect(currentPath()).toBe(pathA);
  expect((automation.start as any).mock.calls.map((row:any)=>row[0].path)).toEqual([pathA,pathB,pathA,pathB,pathA]);
  act(()=>vi.advanceTimersByTime(400));expect(currentPath()).toBe(pathA);
  fireEvent.keyDown(B,{key:'Escape'});expect(currentPath()).toBe(pathA);
  act(()=>vi.advanceTimersByTime(100));expect(held).toBeNull();expect(s.requests.filter(r=>r.method==='transaction.apply')).toEqual([]);
});
