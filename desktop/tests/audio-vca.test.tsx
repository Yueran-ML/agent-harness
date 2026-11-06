// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { VcaAssignment, vcaAssignments } from '../src/studio/fairlight/VcaAssignment';
import { VcaStrip, useVcaGain } from '../src/studio/fairlight/VcaStrip';
import { Mixer } from '../src/studio/fairlight/Mixer';
import { Fader } from '../src/studio/audio/Fader';
import { vcaChoices, vcaOffset } from '../src/studio/audio/routingModel';
import { AutomationTouchProvider, useAutomationTouch } from '../src/studio/fairlight/AutomationTouch';
import { defaultRecord } from '../src/studio/fairlight/automation';
import type { Timeline } from '../src/studio/engine';
import { timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';
import { overlayOpen } from '../src/app/overlay';
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const song = () => ({ ...timeline([]), tracks: ['A1', 'A2'].map(id => ({ id, name: id, kind: 'audio', enabled: true, locked: id === 'A2', clips: [], transitions: [], strip: { fader_db: -8, vcas: ['VCA1'] } })),
  vcas: [{ id: 'VCA1', name: 'Music', fader_db: 3, vcas: ['VCA2'] }, { id: 'VCA2', name: 'Master', fader_db: 2 }] } as unknown as Timeline);
const context = () => { const call = vi.fn(async () => ({ ok: true as const, result: { revision: 8, done: [] } })); return { bridge: { call }, timeline: song(), session: 1, revision: 7 }; };
it('deduplicates VCA ancestors and refuses cycles, ancestor duplicates and a seventh level', () => {
  const t = song(); expect(vcaOffset(t, ['VCA1', 'VCA2'], 0)).toBe(5);
  expect(vcaChoices(t, { vca: 'VCA2' }).find(c => c.vca.id === 'VCA1')?.reason).toBe('vca_cycle');
  expect(vcaChoices(t, { track: 'A1' }).find(c => c.vca.id === 'VCA2')?.reason).toBe('out_of_range');
  t.vcas = Array.from({ length: 7 }, (_, i) => ({ id: `VCA${i+1}`, name: `${i+1}`, vcas: i < 5 ? [`VCA${i+2}`] : [] }));
  expect(vcaChoices(t, { vca: 'VCA6' }).find(c => c.vca.id === 'VCA7')?.reason).toBe('vca_depth');
});
it('creates and assigns a VCA atomically, skips locked targets and removes all membership', () => {
  const t = song();
  const ops = vcaAssignments(t, [{track:'A1'}, {track:'A2'}], {id:'VCA3', name:'VCA 3'});
  expect(ops).toHaveLength(2); expect(ops[0]).toMatchObject({op:'add_vca',vca:{id:'VCA3'}});
  expect(ops[1]).toMatchObject({edit:'set_track_audio',track:'A1',patch:{strip:{vcas:['VCA1','VCA3']}}});
  expect(vcaAssignments(t,[{track:'A1'}],null)).toEqual([{edit:'set_track_audio',timeline:t.id,track:'A1',patch:{strip:{vcas:[]}},follow_groups:false}]);
});
it('offers initial ten assignments and writes the actual project from the label menu', async () => {
  const c = context(); render(<VcaAssignment context={c} of={{track:'A1'}} selected={['track:A1','track:A2']} />);
  fireEvent.contextMenu(screen.getByRole('button',{name:'VCA A1'}));
  expect(screen.getByRole('menuitemcheckbox',{name:'VCA 10'})).toBeDefined();
  fireEvent.click(screen.getByRole('menuitemcheckbox',{name:'VCA 3'}),{altKey:true}); await settle();
  const request = (c.bridge.call.mock.calls as any[])[0][1];
  expect(request.revision).toBe(7); expect(request.ops).toHaveLength(2); expect(request.ops[1].track).toBe('A1');
});
it('applies the source VCA menu action uniformly to a mixed selection', async () => {
  const c=context();const tracks=c.timeline.tracks as any[];tracks[0].strip.vcas=[];tracks[1].locked=false;
  render(<VcaAssignment context={c} of={{track:'A1'}} selected={['track:A1','track:A2']}/>);
  fireEvent.click(screen.getByRole('button',{name:'VCA A1'}));fireEvent.click(screen.getByRole('menuitemcheckbox',{name:'Music'}),{altKey:true});await settle();
  const ops=(c.bridge.call.mock.calls as any[])[0][1].ops;
  expect(ops).toEqual([{edit:'set_track_audio',timeline:c.timeline.id,track:'A1',patch:{strip:{vcas:['VCA1']}},follow_groups:false}]);
  const remove=vcaAssignments(c.timeline,[{track:'A1'},{track:'A2'}],c.timeline.vcas![0],'remove');
  expect(remove).toHaveLength(1);expect(remove[0]).toMatchObject({track:'A2',patch:{strip:{vcas:[]}}});
});
it('portals the assignment menu outside the clipped mixer, keeps it inside the viewport and restores keyboard focus', async () => {
  const c = context();
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function(this: HTMLElement) {
    const popup = this.getAttribute('role') === 'menu';
    return { x: popup ? 0 : 990, y: popup ? 0 : 740, left: popup ? 0 : 990, top: popup ? 0 : 740, width: popup ? 180 : 30, height: popup ? 250 : 20, right: popup ? 180 : 1020, bottom: popup ? 250 : 760, toJSON() {} };
  });
  const { container } = render(<div className="mixer-region" style={{ overflow: 'auto', height: 60 }}><VcaAssignment context={c} of={{ track: 'A1' }} /></div>);
  const trigger = screen.getByRole('button', { name: 'VCA A1' }); trigger.focus(); fireEvent.keyDown(trigger, { key: 'Enter' });
  const menu = screen.getByRole('menu'); expect(container.contains(menu)).toBe(false); expect(menu.parentElement).toBe(document.body); expect(overlayOpen()).toBe(true);
  expect(menu.classList.contains('route-menu')).toBe(true); expect(menu.style.maxHeight).toContain('100vh');
  expect(Number.parseFloat(menu.style.left) + 180).toBeLessThanOrEqual(window.innerWidth - 4);
  expect(Number.parseFloat(menu.style.top) + 250).toBeLessThanOrEqual(window.innerHeight - 4);
  expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'No VCA' }));
  fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' }); expect(document.activeElement).toBe(screen.getByRole('menuitemcheckbox', { name: 'Music' }));
  fireEvent.keyDown(document.activeElement!, { key: 'Escape' }); expect(screen.queryByRole('menu')).toBeNull(); expect(document.activeElement).toBe(trigger); expect(overlayOpen()).toBe(false);
  fireEvent.keyDown(trigger, { key: 'ArrowDown' }); fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' }); fireEvent.keyDown(document.activeElement!, { key: 'Enter' }); await settle();
  expect((c.bridge.call.mock.calls as any[])[0][1].ops[0]).toMatchObject({ track: 'A1', patch: { strip: { vcas: [] } } });
  expect(document.activeElement).toBe(trigger);
});
it('keyboard VCA fader changes the base by .5 dB or Shift .1, and numeric Enter returns focus', async () => {
  const c = context(); const ui = () => <VcaStrip context={c} vca={c.timeline.vcas![0]} seconds={0} selected={false} onSelect={()=>{}}/>; const view = render(ui());
  const cap = screen.getByRole('slider', { name: 'Fader Music' }); cap.focus(); expect(document.activeElement).toBe(cap);
  fireEvent.keyDown(cap, { key: 'ArrowUp' }); fireEvent.keyUp(cap, { key: 'ArrowUp' }); await settle();
  let writes = (c.bridge.call.mock.calls as any[]).filter(([method]) => method === 'transaction.apply');
  expect(writes[0][1].ops).toEqual([{ op: 'set_vca', timeline: c.timeline.id, id: 'VCA1', patch: { fader_db: 3.5 } }]);
  expect(writes.at(-1)[1].gesture.end).toBe(true);
  c.revision = 8; c.timeline = { ...c.timeline, vcas: c.timeline.vcas!.map(v => v.id === 'VCA1' ? { ...v, fader_db: 3.5 } : v) }; view.rerender(ui());
  fireEvent.keyDown(cap, { key: 'ArrowDown', shiftKey: true }); fireEvent.keyUp(cap, { key: 'ArrowDown' }); await settle();
  writes = (c.bridge.call.mock.calls as any[]).filter(([method]) => method === 'transaction.apply'); expect(writes.some(([, p]) => p.ops.some((op: any) => op.patch?.fader_db === 3.4))).toBe(true);
  fireEvent.keyDown(cap, { key: 'Enter' }); const input = screen.getByRole('textbox'); expect(document.activeElement).toBe(input);
  fireEvent.change(input, { target: { value: '6' } }); fireEvent.keyDown(input, { key: 'Enter' }); await settle();
  writes = (c.bridge.call.mock.calls as any[]).filter(([method]) => method === 'transaction.apply'); expect(writes.at(-1)[1].ops).toEqual([{ op: 'set_vca', timeline: c.timeline.id, id: 'VCA1', patch: { fader_db: 4 } }]);
  expect(document.activeElement).toBe(cap); expect(c.timeline.vcas![0].fader_db).toBe(3.5);
});
it('keyboard VCA automation uses touch only and refuses safe or read-only faders', async () => {
  const c = context(); c.timeline.vcas![0].automation_arm = true;
  const ui = (recording: boolean) => <AutomationTouchProvider {...c} enabled recording={recording} playing show={()=>{}} onFailure={()=>{}}><VcaStrip context={c} vca={c.timeline.vcas![0]} seconds={0} selected={false} onSelect={()=>{}}/></AutomationTouchProvider>;
  const view = render(ui(true)); let cap = screen.getByRole('slider', { name: 'Fader Music' }); cap.focus(); fireEvent.keyDown(cap, { key: 'ArrowUp', shiftKey: true }); fireEvent.keyUp(cap, { key: 'ArrowUp' }); await settle();
  expect(c.bridge.call.mock.calls.length).toBeGreaterThan(0); expect((c.bridge.call.mock.calls as any[]).every(([method]) => method === 'automation.touch')).toBe(true);
  expect((c.bridge.call.mock.calls as any[]).at(-1)[1]).toMatchObject({ touching: false, path: 'fader_db', value: 3.1 });
  c.bridge.call.mockClear(); c.timeline.vcas![0].automation_arm = 'safe'; view.rerender(ui(true)); cap = screen.getByRole('slider'); fireEvent.keyDown(cap, { key: 'ArrowUp' }); fireEvent.keyUp(cap, { key: 'ArrowUp' }); await settle(); expect(c.bridge.call).not.toHaveBeenCalled();
  c.timeline.vcas![0].lanes = { fader_db: { kind: 'continuous', scale: 'linear', points: [{ at: { ticks: 0, scale: 48000 }, value: -4 }] } }; view.rerender(ui(false));
  fireEvent.keyDown(cap, { key: 'ArrowDown' }); fireEvent.keyUp(cap, { key: 'ArrowDown' }); fireEvent.keyDown(cap, { key: 'Enter' }); await settle(); expect(screen.queryByRole('textbox')).toBeNull(); expect(c.bridge.call).not.toHaveBeenCalled();
});
it('shows inherited gain but numeric input writes only the member base', () => {
  const commit = vi.fn(); render(<Fader label="Member" value={-8} displayOffset={5} onStart={()=>true} onChange={()=>{}} onEnd={()=>{}} onCancel={()=>{}} onCommit={commit}/>);
  expect(screen.getByRole('slider').getAttribute('aria-valuenow')).toBe('-3');
  fireEvent.doubleClick(screen.getByRole('button',{name:'Member value'}));
  fireEvent.change(screen.getByRole('textbox'),{target:{value:'-2'}}); fireEvent.keyDown(screen.getByRole('textbox'),{key:'Enter'});
  expect(commit).toHaveBeenCalledWith(-7);
});
it('renders the VCA strip in the mixer and moves member caps without changing member bases', () => {
  const c = context(); render(<Mixer {...c} frame={0} meters={null} selected={[]} onSelect={()=>{}}/>);
  expect(screen.getByRole('slider',{name:'Fader A1'}).getAttribute('aria-valuenow')).toBe('-3');
  expect(screen.getByRole('slider',{name:'Fader Music'}).getAttribute('aria-valuenow')).toBe('5');
  expect(c.bridge.call).not.toHaveBeenCalled();
});
it('edits only a VCA with a single gesture and cancels to its base, not the summed value', async () => {
  const c = context(); render(<VcaStrip context={c} vca={c.timeline.vcas![0]} seconds={0} selected={false} onSelect={()=>{}}/>);
  const cap = screen.getByRole('slider',{name:'Fader Music'});
  fireEvent.pointerDown(cap,{button:0,clientY:100}); fireEvent.pointerMove(cap,{clientY:90}); await settle();
  fireEvent.keyDown(cap,{key:'Escape'}); await settle();
  const writes = (c.bridge.call.mock.calls as any[]).filter(x=>x[0]==='transaction.apply');
  expect(writes[0][1].ops[0]).toMatchObject({op:'set_vca',id:'VCA1'});
  expect(writes.at(-1)[1].ops).toEqual([{op:'set_vca',timeline:c.timeline.id,id:'VCA1',patch:{fader_db:3}}]);
  expect(writes.at(-1)[1].gesture).toEqual({id:writes[0][1].gesture.id,end:true});
});
it('never falls back to a static edit when automation recording refuses a safe VCA', async () => {
  const c = context(); c.timeline.vcas![0].automation_arm='safe';
  render(<AutomationTouchProvider {...c} enabled recording playing show={()=>{}} onFailure={()=>{}}><VcaStrip context={c} vca={c.timeline.vcas![0]} seconds={0} selected={false} onSelect={()=>{}}/></AutomationTouchProvider>);
  const cap = screen.getByRole('slider',{name:'Fader Music'}); fireEvent.wheel(cap,{deltaY:-1}); await settle();
  expect(c.bridge.call).not.toHaveBeenCalled();
});
it('records and releases a VCA fader through automation.touch only', async () => {
  const c=context(); c.timeline.vcas![0].automation_arm=true;
  render(<AutomationTouchProvider {...c} enabled recording playing show={()=>{}} onFailure={()=>{}}><VcaStrip context={c} vca={c.timeline.vcas![0]} seconds={0} selected={false} onSelect={()=>{}}/></AutomationTouchProvider>);
  const cap=screen.getByRole('slider',{name:'Fader Music'});
  fireEvent.pointerDown(cap,{button:0,clientY:100});fireEvent.pointerMove(cap,{clientY:90});fireEvent.pointerUp(cap);await settle();
  const calls=c.bridge.call.mock.calls as any[];
  expect(calls.every(([method])=>method==='automation.touch')).toBe(true);
  expect(calls.at(-1)[1]).toMatchObject({vca:'VCA1',path:'fader_db',touching:false});
  expect(calls.at(-1)[1].value).toBeGreaterThan(3);
});
it('does not jump the base when inherited gain puts the displayed cap above its travel', () => {
  const change=vi.fn();render(<Fader label="Member" value={8} displayOffset={5} onStart={()=>true} onChange={change} onEnd={()=>{}} onCancel={()=>{}} onCommit={()=>{}}/>);
  const cap=screen.getByRole('slider');fireEvent.pointerDown(cap,{button:0,clientY:100});fireEvent.pointerMove(cap,{clientY:100});
  expect(change).toHaveBeenCalledWith(8);
});
it('keeps the silence floor when a member or inherited VCA is off', () => {
  const props={label:'Member',onStart:()=>true,onChange:()=>{},onEnd:()=>{},onCancel:()=>{},onCommit:()=>{}};
  const view=render(<Fader {...props} value={-100} displayOffset={6}/>);
  expect(screen.getByRole('slider').getAttribute('aria-valuenow')).toBe('-100');
  view.rerender(<Fader {...props} value={6} displayOffset={-Infinity}/>);
  expect(screen.getByRole('slider').getAttribute('aria-valuenow')).toBe('-100');
});
it('glides a released VCA Trim offset toward zero without adding its base twice', async () => {
  vi.useFakeTimers();localStorage.setItem('fairlight.glide_ms','250');
  const c=context();c.timeline.vcas![0].vcas=[];c.timeline.vcas![0].automation_arm=true;
  c.timeline.audio={...c.timeline.audio!,record:{...defaultRecord(),mode:'trim',touch:'snap',preview:false}};
  function Probe(){const control=useAutomationTouch()!, gain=useVcaGain(c.timeline,['VCA1'],0);return <><output>{gain.toFixed(2)}</output><button onClick={()=>{control.start({of:{vca:'VCA1'},path:'fader_db'},2);control.end();}}>Release Trim</button></>;}
  render(<AutomationTouchProvider {...c} enabled recording playing show={()=>{}} onFailure={()=>{}}><Probe/></AutomationTouchProvider>);
  fireEvent.click(screen.getByText('Release Trim'));expect(screen.getByRole('status').textContent).toBe('5.00');
  await act(async()=>{await vi.advanceTimersByTimeAsync(125);});
  const middle=Number(screen.getByRole('status').textContent);expect(middle).toBeGreaterThan(3.9);expect(middle).toBeLessThan(4.2);
  await act(async()=>{await vi.advanceTimersByTimeAsync(140);});expect(screen.getByRole('status').textContent).toBe('3.00');
  localStorage.removeItem('fairlight.glide_ms');
});
it.each([[8,5,10],[-100,2,-Infinity],[0,-3,-3],[3,2,5]])('matches DSP VCA Trim limits from %s plus %s', (base,offset,expected) => {
  const c=context();Object.assign(c.timeline.vcas![0],{fader_db:base,vcas:[],automation_arm:true});
  c.timeline.audio={...c.timeline.audio!,record:{...defaultRecord(),mode:'trim',touch:'latch',preview:true}};
  function Probe(){const control=useAutomationTouch()!,gain=useVcaGain(c.timeline,['VCA1'],0);return <><output>{String(gain)}</output><button onClick={()=>control.start({of:{vca:'VCA1'},path:'fader_db'},offset)}>Trim</button></>;}
  render(<AutomationTouchProvider {...c} enabled recording show={()=>{}} onFailure={()=>{}}><Probe/></AutomationTouchProvider>);
  fireEvent.click(screen.getByText('Trim'));expect(screen.getByRole('status').textContent).toBe(String(expected));
});
