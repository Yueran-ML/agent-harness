// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Monitoring } from '../src/studio/fairlight/Monitoring';
import { useFairlightView } from '../src/studio/fairlight/viewState';
import type { EngineBridge } from '../src/bridge';
import type { Meters } from '../src/studio/engine';
import { timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';
const patch=vi.fn(),action=vi.fn();
const meters={timeline:'t1',strips:[],control_room:null,loudness:{momentary:-20,momentary_max:-18,short_term:-21,short_term_max:-19,integrated:-23,range:4,true_peak:-1,target:-23,verdict:'within',running:true,seconds:3}} as unknown as Meters;
function Page({bridge,id='t1',session=1}:{bridge:Pick<EngineBridge,'call'>;id?:string;session?:number}) {
  const [view,update,reset]=useFairlightView(bridge,session,id);
  return <><button onClick={reset}>Reset layout</button><Monitoring timeline={{...timeline(),id}} meters={meters} onAction={action} onResetPeaks={()=>{}} onPatch={patch} viewer={<span>Picture</span>}
    view={view.monitoring} onView={monitoring=>update({monitoring})}/></>;
}
beforeEach(()=>{likeABrowser();vi.stubGlobal('PointerEvent',MouseEvent);});
afterEach(()=>{cleanup();vi.restoreAllMocks();vi.unstubAllGlobals();patch.mockClear();action.mockClear();});
it('remembers rows and widths across leaving, isolates timelines and resets only the current view',()=>{
  const bridge={call:vi.fn()};const mount=render(<Page bridge={bridge}/>);
  fireEvent.click(screen.getByText('Two rows'));fireEvent.click(screen.getByText('Narrow meters'));
  fireEvent.keyDown(screen.getByRole('separator',{name:'Loudness width'}),{key:'ArrowRight'});
  mount.unmount();const next=render(<Page bridge={bridge}/>);
  expect(screen.getByRole('region',{name:'Monitoring'}).dataset.rows).toBe('2');
  expect(screen.getByRole('region',{name:'Monitoring'}).dataset.wide).toBe('false');
  expect(screen.getByRole('separator',{name:'Loudness width'}).getAttribute('aria-valuenow')).toBe('316');
  next.rerender(<Page key="t2" bridge={bridge} id="t2"/>);
  expect(screen.getByRole('region',{name:'Monitoring'}).dataset.rows).toBe('1');
  next.rerender(<Page key="t1" bridge={bridge}/>);fireEvent.click(screen.getByText('Reset layout'));
  expect(screen.getByRole('separator',{name:'Loudness width'}).getAttribute('aria-valuenow')).toBe('300');
  fireEvent.click(screen.getByText('Two rows'));next.rerender(<Page key="session2" bridge={bridge} session={2}/>);
  expect(screen.getByRole('region',{name:'Monitoring'}).dataset.rows).toBe('1');
  next.rerender(<Page key="other-bridge" bridge={{call:vi.fn()}}/>);
  expect(screen.getByRole('region',{name:'Monitoring'}).dataset.rows).toBe('1');
  expect(patch).not.toHaveBeenCalled();expect(bridge.call).not.toHaveBeenCalled();
});
it('uses absolute EBU +18 ticks and leaves missing momentary values unavailable',()=>{
  const selected={...timeline(),audio:{loudness:{relative:false,scale:'ebu18',target:-24}}} as ReturnType<typeof timeline>;
  const mount=render(<Monitoring timeline={selected} meters={{...meters,loudness:{...meters.loudness,target:-24}} as Meters} onAction={action} onResetPeaks={()=>{}} onPatch={patch}/>);
  expect(screen.getByLabelText('Loudness ruler').textContent).toContain('−60');expect(screen.getByLabelText('Loudness ruler').textContent).toContain('−6');
  expect(screen.getByRole('meter',{name:'Momentary sum'}).getAttribute('aria-valuenow')).toBe('-20');
  expect(screen.getByLabelText('Momentary maximum').textContent).toBe('-18.0 LUFS');
  mount.rerender(<Monitoring timeline={selected} meters={null} onAction={action} onResetPeaks={()=>{}} onPatch={patch}/>);
  expect(screen.getByRole('meter',{name:'Momentary sum'}).hasAttribute('aria-valuenow')).toBe(false);
  expect(screen.getByLabelText('Momentary maximum').textContent).toBe('−∞ LUFS');
});
it('resizes each visible meter panel with clamped values and the viewer from its actual right edge',()=>{
  render(<Page bridge={{call:vi.fn()}}/>);
  const width=(name:string)=>screen.getByRole('separator',{name});
  fireEvent.pointerDown(width('Viewer width'),{button:0,clientX:400});fireEvent.pointerMove(width('Viewer width'),{clientX:450});fireEvent.pointerUp(width('Viewer width'));
  expect(width('Viewer width').getAttribute('aria-valuenow')).toBe('350');
  expect(document.querySelector('.fairlight-small-viewer')?.nextElementSibling).toBe(width('Viewer width'));
  fireEvent.pointerDown(width('Bus meters width'),{button:0,clientX:400});fireEvent.pointerMove(width('Bus meters width'),{clientX:0});fireEvent.pointerUp(width('Bus meters width'));
  expect(width('Bus meters width').getAttribute('aria-valuenow')).toBe('60');
  fireEvent.keyDown(width('Track meters width'),{key:'ArrowRight'});expect(width('Track meters width').getAttribute('aria-valuenow')).toBe('236');
  expect(patch).not.toHaveBeenCalled();
});
it('shows an actual aggregate momentary bar and its maximum on a labelled EBU scale',()=>{
  render(<Page bridge={{call:vi.fn()}}/>);
  expect(screen.getByLabelText('Momentary maximum').textContent).toContain('5.0 LU');
  expect(screen.getByRole('meter',{name:'Momentary sum'}).getAttribute('aria-valuenow')).toBe('3');
  expect(screen.getByLabelText('Loudness ruler').textContent).toContain('−18');
  expect(screen.getByLabelText('Loudness ruler').textContent).toContain('+9');
});
it('makes the meter context options operate the same view state as the existing buttons',()=>{
  render(<Page bridge={{call:vi.fn()}}/>);
  fireEvent.contextMenu(screen.getByRole('region',{name:'Monitoring'}),{clientX:50,clientY:50});
  fireEvent.click(screen.getByRole('menuitem',{name:'Two rows'}));
  expect(screen.getByRole('region',{name:'Monitoring'}).dataset.rows).toBe('2');
  expect(patch).not.toHaveBeenCalled();
});
it('clears the true peak alone with a click on the Control Room TP readout',()=>{
  render(<Monitoring timeline={timeline()} meters={meters} onAction={action} onResetPeaks={()=>{}} onPatch={patch}/>);
  const peak=screen.getByRole('button',{name:'Reset true peak'});
  expect(peak.textContent).toBe('-1.0 dBTP');
  fireEvent.click(peak);
  expect(action.mock.calls).toEqual([['reset_peak']]);
});
