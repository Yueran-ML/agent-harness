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
import { mount, projectOf, sent, settle } from './colorEngine';
import { media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbina?: unknown }).columbina; });

const rcm = (fields: Partial<ColorSettings> = {}): ColorSettings => ({ ...defaultColorSettings(), science: 'yrgb_managed', ...fields });
const labels = en.label.studio.media;
const m2: Media = { ...media, id: 'm2', name: 'Street', color_override: { input: 'Apple Log', levels: 'video' } };
const sound: Media = { ...media, id: 'm3', facts: { ...media.facts, video: null } };

describe('the colour items of the menu of media', () => {
  it('offer Project Default, Bypass and every space of the catalogue, and Data Levels', () => {
    const items = mediaColorItems([media], rcm(), labels, () => {});
    expect(items.map(each => each.label)).toEqual(['Input Color Space', 'Data Levels']);
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
    expect(items.map(each => each.label)).toEqual(['Input Color Space', 'Input Gamma', 'Data Levels']);
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
    expect(project().media.slice(0, 2).map(each => each.color_override)).toEqual([
      { input: 'DJI D-Gamut/D-Log', levels: 'full' }, { input: 'DJI D-Gamut/D-Log', levels: 'full' },
    ]);
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
