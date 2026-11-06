// @vitest-environment jsdom
/**
 * The small menus of the mixer (a knob's, a strip's Arm and fader menus, the VCA assignment) lie
 * over the window, not inside the strip that clips its overflow: the arrows go along them,
 * Escape or a press elsewhere puts them away, and they stay by their anchor as the page scrolls.
 */
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Knob } from '../src/studio/audio/Knob';
import { AutomationTouchProvider } from '../src/studio/fairlight/AutomationTouch';
import { Mixer } from '../src/studio/fairlight/Mixer';
import { VcaAssignment } from '../src/studio/fairlight/VcaAssignment';
import type { Timeline } from '../src/studio/engine';
import { timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const song = { ...timeline([]), tracks: [{ id: 'a1', name: 'Voice', kind: 'audio', enabled: true, locked: false, clips: [],
  transitions: [], automation_arm: true, strip: { fader_db: 0 } }] } as unknown as Timeline;
const call = vi.fn(async () => ({ ok: true as const, result: { revision: 8, done: [] } }));
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
const touch = (children: React.ReactNode, show = vi.fn()) => <AutomationTouchProvider bridge={{ call }} session={1} timeline={song} enabled
  recording={false} show={show} onFailure={error => { throw error; }}>{children}</AutomationTouchProvider>;

it('lays the Arm and fader menus of a strip over the window, with arrows, Escape and a press elsewhere', async () => {
  const show = vi.fn();
  render(touch(<Mixer bridge={{ call }} session={1} revision={7} timeline={song} frame={0} meters={null} selected={[]} onSelect={() => {}} />, show));
  await settle();
  const strip = document.querySelector('[data-mixer-strip="track:a1"]')!;
  fireEvent.contextMenu(within(strip as HTMLElement).getByRole('button', { name: 'Arm' }), { clientX: 30, clientY: 40 });
  const arm = screen.getByRole('menu', { name: 'Arm Voice' });
  expect(strip.contains(arm)).toBe(false);
  expect(arm.parentElement).toBe(document.body);
  expect(document.activeElement).toBe(within(arm).getByRole('menuitem', { name: 'Safe' }));
  fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
  expect(document.activeElement).toBe(within(arm).getByRole('menuitem', { name: 'Bypass' }));
  fireEvent.keyDown(document.activeElement!, { key: 'End' });
  expect(document.activeElement).toBe(within(arm).getByRole('menuitem', { name: 'Cancel' }));
  fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
  expect(screen.queryByRole('menu')).toBeNull();
  // The fader's menu: a press elsewhere puts it away; its item shows the lane.
  fireEvent.contextMenu(strip.querySelector('.mixer-fader-meter')!, { clientX: 30, clientY: 40 });
  expect(screen.getByRole('menu', { name: 'Fader Voice' }).parentElement).toBe(document.body);
  fireEvent.pointerDown(document.body);
  expect(screen.queryByRole('menu')).toBeNull();
  fireEvent.contextMenu(strip.querySelector('.mixer-fader-meter')!, { clientX: 30, clientY: 40 });
  fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
  expect(show).toHaveBeenCalledWith({ of: { track: 'a1' }, path: 'fader_db' });
  expect(screen.queryByRole('menu')).toBeNull();
});

it("lays a knob's menu over the window and puts it away with Escape", () => {
  const show = vi.fn();
  const view = render(touch(<div data-audio-owner="track:a1" style={{ overflow: 'hidden' }}>
    <Knob path="pan.x" label="Pan" value={0} min={-1} max={1} fallback={0} onStart={() => true} onChange={() => {}} onEnd={() => {}}
      onCancel={() => {}} onCommit={() => {}} />
  </div>, show));
  fireEvent.contextMenu(view.container.querySelector('.audio-knob')!, { clientX: 10, clientY: 10 });
  const menu = screen.getByRole('menu', { name: 'Pan' });
  expect(view.container.contains(menu)).toBe(false);
  fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
  expect(screen.queryByRole('menu')).toBeNull();
  fireEvent.contextMenu(view.container.querySelector('.audio-knob')!, { clientX: 10, clientY: 10 });
  fireEvent.click(screen.getByRole('menuitem', { name: 'Show automation in timeline' }));
  expect(show).toHaveBeenCalledWith({ of: { track: 'a1' }, path: 'pan.x' });
});

it('keeps the VCA menu by its button as a parent scrolls and the window is resized', () => {
  let top = 100;
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const popup = this.getAttribute('role') === 'menu';
    const box = popup ? { left: 0, top: 0, width: 100, height: 50 } : { left: 200, top, width: 30, height: 20 };
    return { ...box, x: box.left, y: box.top, right: box.left + box.width, bottom: box.top + box.height, toJSON() {} };
  });
  const view = render(<div className="mixer-region" style={{ overflow: 'auto' }}>
    <VcaAssignment context={{ bridge: { call } as never, session: 1, revision: 7, timeline: song }} of={{ track: 'a1' }} />
  </div>);
  fireEvent.click(screen.getByRole('button', { name: 'VCA Voice' }));
  const menu = screen.getByRole('menu');
  expect(menu.style.top).toBe('120px');
  top = 60;
  fireEvent.scroll(view.container.querySelector('.mixer-region')!);
  expect(menu.style.top).toBe('80px');
  top = 30;
  fireEvent(window, new Event('resize'));
  expect(menu.style.top).toBe('50px');
});
