// @vitest-environment jsdom
/**
 * The colour of media in the right-click menu of the media pool (Reference Manual 20, pp. 221–222,
 * 228–230, 234–235, 245; the interaction reference, §7.3): Input Color Space ▸ (ACES Input Transform
 * ▸ under ACES, with Input Gamma ▸ where spaces are a gamut and a gamma) and Data Levels ▸, sent for
 * every medium chosen as one transaction; camera raw refused as the engine refuses it.
 */
import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { zhCN } from '../src/messages/zh-CN';
import { en } from '../src/messages/en';
import { defaultColorSettings, type ColorSettings, type Media } from '../src/studio/engine';
import { mediaColorItems } from '../src/studio/media/colorMenu';
import { MEDIA_GAMUTS, SPACES } from '../src/studio/settings/colorSettings';
import { library, mount, projectOf, sent, settle } from './colorEngine';
import { media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbina?: unknown }).columbina; });

const rcm = (fields: Partial<ColorSettings> = {}): ColorSettings => ({ ...defaultColorSettings(), science: 'yrgb_managed', ...fields });
const labels = en.label.studio.media;
const m2: Media = { ...media, id: 'm2', name: 'Street', color_override: { input: 'Apple Log', levels: 'video', lut: { '1d': null, '3d': null }, inverse_drt: true } };
const sound: Media = { ...media, id: 'm3', facts: { ...media.facts, video: null } };

describe('the colour items of the menu of media', () => {
  it('offer Project Default, Bypass and every space of the catalogue, and Data Levels', () => {
    const items = mediaColorItems([media], rcm(), labels, () => {});
    expect(items.map(each => each.label)).toEqual(['Input Color Space', 'Data Levels', 'Inverse DRT for SDR to HDR Conversion']);
    expect(items[0].items!.map(each => each.label)).toEqual(['Project Default', 'Bypass', ...SPACES]);
    expect(items[0].items![0].checked).toBe(true);
    expect(items[1].items!.map(each => [each.label, each.checked])).toEqual([['Auto', true], ['Video', false], ['Full', false]]);
  });

  it('offer only Data Levels where the project is not colour managed, and ACES Input Transform under ACES', () => {
    expect(mediaColorItems([media], defaultColorSettings(), labels, () => {}).map(each => each.label)).toEqual(['Data Levels']);
    expect(mediaColorItems([media], { ...defaultColorSettings(), science: 'acescc' }, labels, () => {}).map(each => each.label))
      .toEqual(['ACES Input Transform', 'Data Levels']);
  });

  it('send one transaction for every medium with a picture, leaving out those that have it already', () => {
    const run = vi.fn();
    const items = mediaColorItems([media, m2, sound], rcm(), labels, run);
    // The two differ: nothing is checked.
    expect(items[0].items!.some(each => each.checked)).toBe(false);
    items[0].items!.find(each => each.label === 'Apple Log')!.run!();
    items[1].items!.find(each => each.label === 'Video')!.run!();
    items[0].items![0].run!();
    expect(run.mock.calls).toEqual([
      [[{ op: 'set_media_color', id: 'm1', input: 'Apple Log' }]],
      [[{ op: 'set_media_color', id: 'm1', levels: 'video' }]],
      [[{ op: 'set_media_color', id: 'm2', input: null }]],
    ]);
    // Media of sound alone has nothing to set.
    expect(mediaColorItems([sound], rcm(), labels, run).every(each => each.disabled)).toBe(true);
  });

  it('offer a gamut and a gamma apart where the project uses them apart', () => {
    const run = vi.fn();
    const items = mediaColorItems([media, m2], rcm({ preset: 'custom', separate_gamma: true, input: 'P3-D65 Gamma 2.6' }), labels, run);
    expect(items.map(each => each.label)).toEqual(['Input Color Space', 'Input Gamma', 'Data Levels', 'Inverse DRT for SDR to HDR Conversion']);
    expect(items[0].items!.slice(2, 8).map(each => each.label)).toEqual([...MEDIA_GAMUTS]);
    // The gammas of the first medium's gamut: the project's input, P3-D65.
    expect(items[1].items!.map(each => each.label)).toContain('ST2084');
    items[0].items!.find(each => each.label === 'Rec.2020')!.run!();
    items[1].items!.find(each => each.label === 'HLG')!.run!();
    expect(run.mock.calls).toEqual([
      // Each keeps its gamma where the gamut has it: the project's Gamma 2.6, and Apple Log's own, which Rec.2020 has not.
      [[{ op: 'set_media_color', id: 'm1', input: 'Rec.2020 Gamma 2.6' }, { op: 'set_media_color', id: 'm2', input: 'Rec.2020 Gamma 2.4' }]],
      // Apple Log has no HLG: only the first is set.
      [[{ op: 'set_media_color', id: 'm1', input: 'P3-D65 HLG' }]],
    ]);
  });
});

const item = (id: string) => document.querySelector<HTMLElement>(`[data-test="pool-item"][data-media="${id}"]`)!;

describe('the menu of the media pool', () => {
  it('sets the input colour space and the data levels of the media chosen, as one transaction each', async () => {
    const { bridge, project } = await mount(projectOf({ color: rcm() }));
    fireEvent.click(item('m1'));
    fireEvent.click(item('m2'), { ctrlKey: true });
    bridge.call.mockClear();
    fireEvent.contextMenu(item('m2'), { clientX: 40, clientY: 40 });
    const menu = screen.getByRole('menu', { name: 'Media menu' });
    // The menu lies over the viewer: the view is hidden while it is open.
    expect(document.body.dataset.overlay).toBe('1');
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Input Color Space' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'DJI D-Gamut/D-Log' }));
    await settle();
    fireEvent.contextMenu(item('m1'), { clientX: 40, clientY: 40 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Data Levels' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Full' }));
    await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'set_media_color', id: 'm1', input: 'DJI D-Gamut/D-Log' }, { op: 'set_media_color', id: 'm2', input: 'DJI D-Gamut/D-Log' }],
      [{ op: 'set_media_color', id: 'm1', levels: 'full' }, { op: 'set_media_color', id: 'm2', levels: 'full' }],
    ]);
    const kept = { input: 'DJI D-Gamut/D-Log', levels: 'full', lut: { '1d': null, '3d': null }, inverse_drt: true };
    expect(project().media.slice(0, 2).map(each => each.color_override)).toEqual([kept, kept]);
    // Both have it now: it is checked.
    fireEvent.contextMenu(item('m1'), { clientX: 40, clientY: 40 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Input Color Space' }));
    expect(screen.getByRole('menuitem', { name: 'DJI D-Gamut/D-Log' }).dataset.checked).toBe('true');
    expect(document.body.dataset.overlay).toBe('1');
  });

  it('chooses the medium right-clicked outside what is chosen, alone', async () => {
    const { bridge } = await mount(projectOf({ color: rcm() }));
    fireEvent.click(item('m1'));
    bridge.call.mockClear();
    fireEvent.contextMenu(item('m2'), { clientX: 40, clientY: 40 });
    expect(item('m2').getAttribute('aria-selected')).toBe('true');
    expect(item('m1').getAttribute('aria-selected')).toBe('false');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Input Color Space' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Bypass' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_media_color', id: 'm2', input: 'bypass' }]]);
  });

  it('says that the space of camera raw is its camera\'s, as the engine refuses it', async () => {
    const { refuse } = await mount(projectOf({ color: rcm() }), { words: zhCN });
    refuse({ reason: 'camera_raw', media: 'm1' });
    fireEvent.contextMenu(item('m1'), { clientX: 40, clientY: 40 });
    fireEvent.click(screen.getByRole('menuitem', { name: '输入色彩空间' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Apple Log' }));
    await settle();
    expect(screen.getByRole('alert').textContent).toContain('摄影机 RAW 媒体的色彩空间由摄影机决定，不能手动设置。');
  });

  it('is not offered outside Windows', async () => {
    (window as { columbina?: unknown }).columbina = { platform: 'darwin' };
    await mount(projectOf({ color: rcm() }));
    fireEvent.contextMenu(item('m1'), { clientX: 40, clientY: 40 });
    expect(screen.queryByRole('menu', { name: 'Media menu' })).toBeNull();
  });
});

describe('the LUTs and the Inverse DRT of a source clip', () => {
  const lutWords = en.label.studio.colorPage.lutPanel;
  const luts = { library: library(), favorites: new Set(['Film/Kodak 2383.cube']), words: lutWords };

  it('are set by 1D LUT and 3D LUT, None taking each off, for every medium chosen, and the Inverse DRT under RCM alone', () => {
    const run = vi.fn();
    const withLut: Media = { ...media, id: 'm4', color_override: { input: null, levels: 'auto', lut: { '1d': null, '3d': 'Looks/Teal.cube' }, inverse_drt: false } };
    const items = mediaColorItems([media, withLut], rcm(), labels, run, luts);
    expect(items.map(each => each.label)).toEqual(['Input Color Space', 'Data Levels', '1D LUT', '3D LUT', 'Inverse DRT for SDR to HDR Conversion']);
    const three = items.find(each => each.id === 'media-lut-3d')!;
    // Not the same for both: nothing is checked; the LUTs of 3D, and the favorites of 3D.
    expect(three.items!.map(each => [each.label, each.checked ?? false])).toEqual([
      ['None', false], ['shaper', false], ['Film', false], ['Looks', false], ['Favorites', false],
    ]);
    three.items!.find(each => each.label === 'Film')!.items!.find(each => each.label === 'Kodak 2383')!.run!();
    expect(run).toHaveBeenLastCalledWith([
      { op: 'set_media_color', id: 'm1', lut: { '1d': null, '3d': 'Film/Kodak 2383.cube' } },
      { op: 'set_media_color', id: 'm4', lut: { '1d': null, '3d': 'Film/Kodak 2383.cube' } },
    ]);
    // None takes it off where there is one; the 1D is set beside the 3D that is there.
    three.items![0].run!();
    expect(run).toHaveBeenLastCalledWith([{ op: 'set_media_color', id: 'm4', lut: { '1d': null, '3d': null } }]);
    expect(items.find(each => each.id === 'media-lut-1d')!.items!.map(each => each.label)).toEqual(['None', 'Favorites']);
    // The Inverse DRT: not on for both, so unchecked, and turned on for the one that has it off.
    const inverse = items.find(each => each.id === 'inverse-drt')!;
    expect(inverse.checked).toBe(false);
    inverse.run!();
    expect(run).toHaveBeenLastCalledWith([{ op: 'set_media_color', id: 'm4', inverse_drt: true }]);
    const on = mediaColorItems([media], rcm(), labels, run, luts).find(each => each.id === 'inverse-drt')!;
    expect(on.checked).toBe(true);
    on.run!();
    expect(run).toHaveBeenLastCalledWith([{ op: 'set_media_color', id: 'm1', inverse_drt: false }]);
    // Not under YRGB or ACES, where there is no inverse DRT; the LUTs are there in every science.
    for (const science of ['yrgb', 'acescct'] as const) {
      const others = mediaColorItems([media], { ...defaultColorSettings(), science }, labels, run, luts);
      expect(others.some(each => each.id === 'inverse-drt')).toBe(false);
      expect(others.some(each => each.id === 'media-lut-3d')).toBe(true);
    }
    // Media without a picture has none.
    const silent = mediaColorItems([sound], rcm(), labels, run, luts);
    expect(silent.filter(each => each.disabled).map(each => each.id)).toEqual(['input-space', 'data-levels', 'media-lut-1d', 'media-lut-3d', 'inverse-drt']);
  });

  it('are in the menu of the media pool and of a thumbnail of the Color page, one transaction for those chosen', async () => {
    const { bridge, project } = await mount(projectOf({ color: rcm() }), { page: 'Color' });
    // The thumbnail of the first clip, of m1.
    fireEvent.contextMenu(document.querySelector('.color-thumb[data-clip="c1"]')!, { clientX: 40, clientY: 40 });
    fireEvent.click(screen.getByRole('menuitem', { name: '3D LUT' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Looks' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Teal' }));
    await settle();
    expect(sent(bridge).at(-1)).toEqual([{ op: 'set_media_color', id: 'm1', lut: { '1d': null, '3d': 'Looks/Teal.cube' } }]);
    expect(project().media[0].color_override?.lut).toEqual({ '1d': null, '3d': 'Looks/Teal.cube' });
    fireEvent.contextMenu(document.querySelector('.color-thumb[data-clip="c1"]')!, { clientX: 40, clientY: 40 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Inverse DRT for SDR to HDR Conversion' }));
    await settle();
    expect(sent(bridge).at(-1)).toEqual([{ op: 'set_media_color', id: 'm1', inverse_drt: false }]);
  });
});
