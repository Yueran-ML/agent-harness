// @vitest-environment jsdom
/**
 * What the studio says and asks of the engine's colour beyond the palettes (wave E): the Color
 * Viewer LUT on the Color page alone (Reference Manual 20, p. 672; `viewer.lut`), the colour
 * management being made or not made (`color_pending`, `color_error` of the timeline state), and
 * the settings of the Color Management panel that the engine keeps but does not do yet, greyed and
 * said so (pp. 243, 250).
 */
import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { defaultColorSettings, type ColorSettings } from '../src/studio/engine';
import { calls, mount, projectOf, settle } from './colorEngine';
import { likeABrowser } from './mounted';

const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser();
  localStorage.clear();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('color-page')) return rect(0, 0, 1600, 900);
    return rect(0, 0, 600, 300);
  });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbina?: unknown }).columbina; localStorage.clear(); });

const tab = (name: string) => screen.getByRole('tab', { name });

describe('the Color Viewer LUT', () => {
  const lutted = { ...defaultColorSettings(), viewer_lut: { '1d': null, '3d': 'Looks/Teal.cube' } };

  it('is on the Color page alone, told to the engine as the page changes where the project has one', async () => {
    const { bridge, restart } = await mount(projectOf({ color: lutted }));
    expect(calls(bridge, 'viewer.lut')).toEqual([{ on: false }]);
    fireEvent.click(tab('Color'));
    await settle();
    fireEvent.click(tab('Edit'));
    await settle();
    fireEvent.click(tab('Cut'));
    await settle();
    expect(calls(bridge, 'viewer.lut')).toEqual([{ on: false }, { on: true }, { on: false }]);
    // Another engine shows it at first: on the Color page there is nothing to tell it; elsewhere, off.
    fireEvent.click(tab('Color'));
    await settle();
    await restart();
    await settle();
    expect(calls(bridge, 'viewer.lut')).toEqual([{ on: false }, { on: true }, { on: false }, { on: true }]);
    fireEvent.click(tab('Edit'));
    await settle();
    await restart();
    await settle();
    expect(calls(bridge, 'viewer.lut')).toEqual([{ on: false }, { on: true }, { on: false }, { on: true }, { on: false }, { on: false }]);
  });

  it('is not told of where the project has none, until one is set off the Color page', async () => {
    const { bridge, outside } = await mount(projectOf());
    fireEvent.click(tab('Color'));
    await settle();
    fireEvent.click(tab('Edit'));
    await settle();
    expect(calls(bridge, 'viewer.lut')).toEqual([]);
    await outside([{ op: 'set_color_settings', settings: { viewer_lut: { '3d': 'Looks/Teal.cube' } } }]);
    await settle();
    expect(calls(bridge, 'viewer.lut')).toEqual([{ on: false }]);
  });
});

describe('the colour management being made', () => {
  it('is said under the timeline viewer while it is, and where it cannot be made, why', async () => {
    const { notify } = await mount(projectOf(), { page: 'Color' });
    const viewer = () => screen.getByRole('region', { name: 'Timeline viewer' });
    expect(within(viewer()).queryByTestId('colour-pending')).toBeNull();
    await notify({ color_pending: true });
    expect(within(viewer()).getByRole('status', { name: '' }).textContent).toContain('Applying color management');
    await notify({ color_pending: false });
    expect(document.querySelector('[data-test="colour-pending"]')).toBeNull();
    await notify({ color_error: 'OCIO: no such space' });
    const failed = document.querySelector<HTMLElement>('[data-test="colour-failed"]')!;
    expect(failed.getAttribute('role')).toBe('alert');
    expect(failed.textContent).toContain('OCIO: no such space');
    // On the Edit page too, which shows the same monitor.
    fireEvent.click(tab('Edit'));
    await settle();
    expect(document.querySelector('[data-test="colour-failed"]')).not.toBeNull();
    await notify({ color_error: undefined });
    expect(document.querySelector('[data-test="colour-failed"]')).toBeNull();
  });
});

describe('the settings that the engine does not do yet', () => {
  it('are shown greyed and said so: color space aware tools, resize transformations, and node LUTs of ACES', async () => {
    const custom: ColorSettings = { ...defaultColorSettings(), science: 'yrgb_managed', preset: 'custom' };
    await mount(projectOf({ color: custom }), { page: 'Color' });
    fireEvent.click(screen.getByRole('button', { name: 'Project Settings' }));
    await settle();
    const dialog = screen.getByRole('dialog', { name: 'Project Settings' });
    const aware = within(dialog).getByRole('checkbox', { name: /Use color space aware grading tools/ }) as HTMLInputElement;
    expect(aware.disabled).toBe(true);
    expect(dialog.querySelector('[data-test="not-yet-color_space_aware_tools"]')!.textContent).toContain('Not in effect yet');
    expect((dialog.querySelector('[data-test="color-resize_in"]') as HTMLSelectElement).disabled).toBe(true);
    expect(dialog.querySelector('[data-test="not-yet-resize_in"]')).not.toBeNull();
    // What is done is not greyed.
    expect((dialog.querySelector('[data-test="color-white_point_adaptation"]') as HTMLInputElement).disabled).toBe(false);
    expect(dialog.querySelector('[data-test="not-yet-white_point_adaptation"]')).toBeNull();
    // Under ACES: Process Node LUTs in.
    fireEvent.change(dialog.querySelector('[data-test="color-science"]')!, { target: { value: 'acescct' } });
    await settle();
    expect((dialog.querySelector('[data-test="color-node_luts_in"]') as HTMLSelectElement).disabled).toBe(true);
    expect(dialog.querySelector('[data-test="not-yet-node_luts_in"]')).not.toBeNull();
  });
});
