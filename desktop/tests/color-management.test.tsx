// @vitest-environment jsdom
/**
 * The Color Management panel of the Project Settings (Reference Manual 20, pp. 149–153, 224–252; the
 * interaction reference, §7.1–7.2), Display HDR On Viewers If Available (p. 244) and the viewer's
 * Viewer LUT: which settings are shown for which, the draft that Save sends (one `set_color_settings`
 * of what differs from the project then, with the output of a preset and the ODT that a version of
 * ACES needs) and Cancel forgets, what changed elsewhere meanwhile, what the engine refuses, the
 * LUTs of each kind and Open LUT Folder, and the choice of HDR kept on this computer and sent to
 * each engine as it is opened.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { zhCN } from '../src/messages/zh-CN';
import { defaultColorSettings, type ColorSettings } from '../src/studio/engine';
import {
  acesVersionPatch, CAMERA_SPACES, colorFieldsShown, draftChanges, drafted, gammasOf, isHdrSpace, joinSpace, MEDIA_GAMMAS, MEDIA_GAMUTS, ODTS,
  presetPatch, processingPatch, projectInput, separateGamma, SPACES, splitSpace, withEdit, withGamut,
} from '../src/studio/settings/colorSettings';
import { calls, library, mixedLibrary, mount, projectOf, sent, settle } from './colorEngine';
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
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbina?: unknown }).columbina; localStorage.clear(); });

const settings = (fields: Partial<ColorSettings> = {}): ColorSettings => ({ ...defaultColorSettings(), ...fields });
const rcm = (fields: Partial<ColorSettings> = {}) => settings({ science: 'yrgb_managed', ...fields });
const custom = (fields: Partial<ColorSettings> = {}) => rcm({ preset: 'custom', ...fields });
const LUTS = ['input_lut', 'output_lut', 'viewer_lut', 'lut_interpolation'];

describe('the settings of the panel', () => {
  it('are the spaces and the ODTs of the engine, one for one', () => {
    const model = readFileSync(join(process.cwd(), '../engine/crates/model/src/color_management.rs'), 'utf8');
    const color = readFileSync(join(process.cwd(), '../engine/crates/model/src/color.rs'), 'utf8');
    const block = (from: string) => model.slice(model.indexOf(from), model.indexOf('];', model.indexOf(from)));
    const strings = (text: string) => [...text.matchAll(/"([^"]+)"/g)].map(match => match[1]);
    expect(strings(block('pub const CAMERA_SPACES'))).toEqual([...CAMERA_SPACES]);
    // The test runs in the folder of the desktop app. An ODT is the first string of each `view(...)`, or the `name` of an `Odt { … }`.
    const odts = (from: string) => [...block(from).matchAll(/view\(\s*"([^"]+)"|name: "([^"]+)"/g)].map(match => match[1] ?? match[2]);
    expect(odts('const ODTS_1_3')).toEqual(ODTS['1.3']);
    expect(odts('const ODTS_2_0')).toEqual(ODTS['2.0']);
    // The gamuts and the gammas of media, as `Gamut::name` and `Gamma::name` name them.
    const names = (of: string) => strings(color.slice(color.indexOf(`impl ${of} {`)).split('pub fn name(self)')[1].split('\n    }\n')[0]);
    expect(names('Gamut').filter(name => name !== 'Camera Native')).toEqual([...MEDIA_GAMUTS]);
    expect(names('Gamma')).toEqual([...MEDIA_GAMMAS]);
    expect(SPACES).toHaveLength(68);
    expect(SPACES).toContain('Rec.2100 HLG');
    expect(new Set(SPACES).size).toBe(68);
  });

  it('show under DaVinci YRGB only the science and the lookup tables', () => {
    expect(colorFieldsShown(settings())).toEqual(['science', ...LUTS]);
  });

  it('show the preset and the output under Resolve color management, the mode alone where it is automatic', () => {
    expect(colorFieldsShown(rcm())).toEqual(['science', 'automatic', 'preset', 'output', 'graphics_white', ...LUTS]);
    expect(colorFieldsShown(rcm({ automatic: true }))).toEqual(['science', 'automatic', 'processing_mode', 'output', ...LUTS]);
  });

  it('show the rest under Custom, each where it means something', () => {
    expect(colorFieldsShown(custom())).toEqual([
      'science', 'automatic', 'preset', 'output', 'separate_gamma', 'input', 'timeline', 'working_luminance', 'limit_gamut', 'input_drt', 'output_drt',
      'inverse_drt', 'white_point_adaptation', 'color_space_aware_tools', 'resize_in', 'graphics_white', ...LUTS,
    ]);
    const shown = (fields: Partial<ColorSettings>) => colorFieldsShown(custom(fields));
    expect(shown({ working_luminance: 'custom' })).toContain('custom_luminance');
    expect(shown({ working_luminance: 'hdr_1000' })).not.toContain('custom_luminance');
    expect(shown({ input_drt: 'saturation_preserving' })).toContain('input_sat_rolloff');
    expect(shown({ input_drt: 'saturation_preserving' })).not.toContain('output_sat_rolloff');
    expect(shown({ output_drt: 'saturation_preserving' })).toContain('output_sat_rolloff');
    // The 203 nits reference is shown only for an HDR output of an SDR timeline (p. 239).
    expect(shown({ output: 'Rec.2100 ST2084' })).toContain('hdr_203_nits');
    expect(shown({ output: 'Rec.2100 HLG', timeline: 'DaVinci Wide Gamut/DaVinci Intermediate' })).toContain('hdr_203_nits');
    expect(shown({ output: 'Rec.2100 ST2084', timeline: 'P3-D65 ST2084' })).not.toContain('hdr_203_nits');
    expect(shown({ output: 'Rec.709 Gamma 2.4' })).not.toContain('hdr_203_nits');
    expect(shown({ output: 'bypass' })).not.toContain('hdr_203_nits');
    // A preset other than Custom keeps them hidden, whatever they hold.
    expect(colorFieldsShown(rcm({ working_luminance: 'custom', output: 'Rec.2100 ST2084' }))).not.toContain('hdr_203_nits');
  });

  it('show the five of ACES under either science of ACES', () => {
    for (const science of ['acescct', 'acescc'] as const) {
      expect(colorFieldsShown(settings({ science }))).toEqual(['science', 'aces_version', 'idt', 'odt', 'gamut_compress', 'node_luts_in', ...LUTS]);
    }
  });

  it('send the output of a preset with it, and of a processing mode', () => {
    expect(presetPatch('hdr_rec2020')).toEqual({ preset: 'hdr_rec2020', output: 'Rec.2100 ST2084' });
    expect(presetPatch('sdr_p3_cinema')).toEqual({ preset: 'sdr_p3_cinema', output: 'DCI-P3 Gamma 2.6' });
    expect(presetPatch('davinci_wide_gamut')).toEqual({ preset: 'davinci_wide_gamut', output: 'Rec.709 Gamma 2.4' });
    expect(presetPatch('custom')).toEqual({ preset: 'custom' });
    expect(processingPatch('hdr')).toEqual({ processing_mode: 'hdr', output: 'Rec.2100 ST2084' });
  });

  it('send an ODT with a version of ACES that lacks the one chosen', () => {
    expect(acesVersionPatch('2.0', 'Rec.709 (D60 sim.)')).toEqual({ aces_version: '2.0', odt: 'Rec.709' });
    expect(acesVersionPatch('1.3', 'Rec.2020 ST2084 (500 nits)')).toEqual({ aces_version: '1.3', odt: 'Rec.709' });
    expect(acesVersionPatch('2.0', 'sRGB')).toEqual({ aces_version: '2.0' });
    for (const version of ['1.3', '2.0'] as const) expect(ODTS[version]).toContain('Rec.709');
  });

  it('are edited in a draft, a field of a field apart, and Save sends what differs from the project then', () => {
    let draft = withEdit({}, { output_sat_rolloff: { limit: 2000 }, viewer_lut: { '3d': 'a.cube' } });
    draft = withEdit(draft, { viewer_lut: { '1d': 'b.cube' }, limit_gamut: null, science: 'acescct' });
    expect(draft).toEqual({ output_sat_rolloff: { limit: 2000 }, viewer_lut: { '3d': 'a.cube', '1d': 'b.cube' }, limit_gamut: null, science: 'acescct' });
    // Shown over the project now: what the draft does not touch is the project's.
    const now = settings({ limit_gamut: 'rec709', viewer_lut: { '1d': 'b.cube', '3d': null }, output_sat_rolloff: { start: 150, limit: 1000 } });
    const shown = drafted(now, draft);
    expect(shown.output_sat_rolloff).toEqual({ start: 150, limit: 2000 });
    expect(shown.viewer_lut).toEqual({ '1d': 'b.cube', '3d': 'a.cube' });
    expect(shown.limit_gamut).toBeNull();
    expect(now.output_sat_rolloff).toEqual({ start: 150, limit: 1000 });
    // Only what differs: the 1D LUT is the project's already.
    expect(draftChanges(now, draft)).toEqual({ output_sat_rolloff: { limit: 2000 }, viewer_lut: { '3d': 'a.cube' }, limit_gamut: null, science: 'acescct' });
    expect(draftChanges(shown, draft)).toBeNull();
    expect(draftChanges(now, {})).toBeNull();
  });

  it('cut a space into a gamut and a gamma and put it back, for Use Separate Color Space and Gamma', () => {
    expect(splitSpace('P3-D65 ST2084')).toEqual({ gamut: 'P3-D65', gamma: 'ST2084' });
    expect(splitSpace('sRGB')).toEqual({ gamut: 'Rec.709', gamma: 'sRGB' });
    expect(splitSpace('Rec.2100 HLG')).toEqual({ gamut: 'Rec.2020', gamma: 'HLG' });
    expect(splitSpace('Sony S-Gamut3.Cine/S-Log3')).toEqual({ gamut: 'Sony S-Gamut3.Cine', gamma: 'S-Log3' });
    expect(splitSpace('Apple Log')).toEqual({ gamut: 'Apple Log', gamma: 'Apple Log' });
    for (const space of SPACES) { const { gamut, gamma } = splitSpace(space); expect(joinSpace(gamut, gamma)).toBe(space); }
    expect(gammasOf('Canon Cinema Gamut')).toEqual(['Canon Log 2', 'Canon Log 3']);
    expect(gammasOf('Rec.2020')).toHaveLength(8);
    expect(withGamut('Rec.709 ST2084', 'P3-D65')).toBe('P3-D65 ST2084');
    expect(withGamut('Rec.709 Gamma 2.4', 'Sony S-Gamut3')).toBe('Sony S-Gamut3/S-Log3');
    expect(withGamut('bypass', 'Rec.2020')).toBe('Rec.2020 Gamma 2.4');
    expect(separateGamma(custom({ separate_gamma: true }))).toBe(true);
    expect(separateGamma(rcm({ separate_gamma: true }))).toBe(false);
    expect(isHdrSpace('P3-D65 HLG')).toBe(true);
    expect(isHdrSpace('DaVinci Wide Gamut/DaVinci Intermediate')).toBe(false);
    expect(projectInput(custom({ input: 'Apple Log' }))).toBe('Apple Log');
    expect(projectInput(rcm({ input: 'Apple Log' }))).toBe('Rec.709 Gamma 2.4');
    expect(projectInput(settings({ science: 'acescct', idt: 'ARRI Wide Gamut 4/LogC4' }))).toBe('ARRI Wide Gamut 4/LogC4');
  });
});

const open = async () => { fireEvent.click(screen.getByRole('button', { name: 'Project Settings' })); await settle(); };
const field = (name: string) => document.querySelector<HTMLSelectElement & HTMLInputElement>(`[data-test="color-${name}"]`);
const choose = async (name: string, value: string) => { fireEvent.change(field(name)!, { target: { value } }); await settle(); };
const settingsSent = (bridge: Parameters<typeof sent>[0]) => sent(bridge).map(ops => ops.map(op => (op as { settings: unknown }).settings));
const dialog = () => screen.queryByRole('dialog', { name: 'Project Settings' });
const save = async (alt = false) => { fireEvent.click(screen.getByRole('button', { name: 'Save' }), { altKey: alt }); await settle(); };

describe('the Color Management panel', () => {
  it('opens from the gear of the studio, shows what applies as it is changed, and sends nothing until Save', async () => {
    const { bridge, project } = await mount();
    await open();
    expect(within(dialog()!).getByRole('button', { name: 'Color Management' }).getAttribute('aria-pressed')).toBe('true');
    // DaVinci YRGB: the science and the lookup tables.
    expect(field('science')!.value).toBe('yrgb');
    expect([...field('science')!.options].map(each => each.textContent)).toEqual(['DaVinci YRGB', 'DaVinci YRGB Color Managed', 'DaVinci ACEScc', 'DaVinci ACEScct']);
    expect(field('preset')).toBeNull();
    expect(field('output_lut-3d')).not.toBeNull();
    // Cancel is what has the keyboard: the answer that changes nothing.
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Cancel' }));
    bridge.call.mockClear();
    await choose('science', 'yrgb_managed');
    // The draft shows what applies to it; the project is as it was.
    expect(project().color).toBeUndefined();
    expect(field('preset')!.value).toBe('sdr_rec709');
    expect(field('timeline')).toBeNull();
    await choose('preset', 'hdr_rec2020');
    expect(field('output')!.value).toBe('Rec.2100 ST2084');
    await choose('preset', 'custom');
    expect(field('timeline')).not.toBeNull();
    // An HDR output of an SDR timeline: the 203 nits reference is shown.
    expect(field('hdr_203_nits')).not.toBeNull();
    await choose('timeline', 'Rec.2100 HLG');
    expect(field('hdr_203_nits')).toBeNull();
    await choose('working_luminance', 'custom');
    const nits = field('custom_luminance')!;
    fireEvent.change(nits, { target: { value: '600' } });
    fireEvent.keyDown(nits, { key: 'Enter' });
    await settle();
    // Escape puts a number back.
    fireEvent.change(nits, { target: { value: '900' } });
    fireEvent.keyDown(nits, { key: 'Escape' });
    expect(nits.value).toBe('600');
    fireEvent.blur(nits);
    await choose('output_drt', 'saturation_preserving');
    fireEvent.change(field('output_sat_rolloff-limit')!, { target: { value: '2000' } });
    fireEvent.blur(field('output_sat_rolloff-limit')!);
    await settle();
    fireEvent.click(field('white_point_adaptation')!);
    await settle();
    expect(field('white_point_adaptation')!.checked).toBe(false);
    expect(sent(bridge)).toEqual([]);
    // Save: one transaction of one set_color_settings, with what differs from the project, and the window closes.
    await save();
    expect(settingsSent(bridge)).toEqual([[{
      science: 'yrgb_managed', preset: 'custom', output: 'Rec.2100 ST2084', timeline: 'Rec.2100 HLG', working_luminance: 'custom',
      custom_luminance: 600, output_drt: 'saturation_preserving', output_sat_rolloff: { limit: 2000 }, white_point_adaptation: false,
    }]]);
    expect(project().color).toMatchObject({ science: 'yrgb_managed', preset: 'custom', output_sat_rolloff: { start: 100, limit: 2000 } });
    expect(dialog()).toBeNull();
  });

  it('forgets the draft on Cancel, on Escape and on the cross', async () => {
    const { bridge, project } = await mount();
    bridge.call.mockClear();
    for (const close of [
      () => fireEvent.click(screen.getByRole('button', { name: 'Cancel' })),
      () => fireEvent(dialog()!, new Event('cancel', { cancelable: true })),
      () => fireEvent.click(within(dialog()!).getByRole('button', { name: 'Close' })),
    ]) {
      await open();
      // The draft of the last time is gone.
      expect(field('science')!.value).toBe('yrgb');
      await choose('science', 'acescct');
      fireEvent.click(field('display_hdr')!);
      close();
      await settle();
      expect(dialog()).toBeNull();
    }
    expect(sent(bridge)).toEqual([]);
    expect(calls(bridge, 'view.display')).toEqual([]);
    expect(project().color).toBeUndefined();
  });

  it('keeps the window open on Alt+click of Save, and sends nothing for nothing changed', async () => {
    const { bridge, project } = await mount();
    await open();
    bridge.call.mockClear();
    await choose('lut_interpolation', 'tetrahedral');
    await save(true);
    expect(settingsSent(bridge)).toEqual([[{ lut_interpolation: 'tetrahedral' }]]);
    expect(project().color?.lut_interpolation).toBe('tetrahedral');
    expect(dialog()).not.toBeNull();
    expect(field('lut_interpolation')!.value).toBe('tetrahedral');
    // Chosen back and forth, a setting is as the project has it: nothing to send.
    await choose('lut_interpolation', 'trilinear');
    await choose('lut_interpolation', 'tetrahedral');
    await save();
    expect(sent(bridge)).toHaveLength(1);
    expect(dialog()).toBeNull();
  });

  it('shows what changed elsewhere while it is open, and keeps what was changed in it', async () => {
    const { bridge, outside } = await mount(projectOf({ color: rcm() }));
    await open();
    bridge.call.mockClear();
    await choose('preset', 'sdr_p3_broadcast');
    fireEvent.change(field('graphics_white')!, { target: { value: '200' } });
    fireEvent.blur(field('graphics_white')!);
    await choose('lut_interpolation', 'tetrahedral');
    expect(screen.queryByRole('status')).toBeNull();
    // An agent (or an undo) changes the project meanwhile.
    await outside([{ op: 'set_color_settings', settings: { graphics_white: 150, lut_interpolation: 'tetrahedral', output_lut: { '3d': 'Looks/Teal.cube' }, automatic: false } }]);
    await settle();
    expect(screen.getByRole('status').textContent).toBe('The settings of the project were changed elsewhere. What you have not changed here shows the new values.');
    // What was not changed here shows the new values; what was, the draft's.
    expect(field('output_lut-3d')!.value).toBe('Looks/Teal.cube');
    expect(field('graphics_white')!.value).toBe('200');
    expect(field('preset')!.value).toBe('sdr_p3_broadcast');
    await save();
    // Only what still differs: the interpolation is the project's now.
    expect(settingsSent(bridge)).toEqual([[{ preset: 'sdr_p3_broadcast', output: 'P3-D65 Gamma 2.4', graphics_white: 200 }]]);
  });

  it('says what the engine refused at the top, and stays open with the draft', async () => {
    const { bridge, refuse, project } = await mount(projectOf({ color: settings({ science: 'acescct' }) }));
    await open();
    bridge.call.mockClear();
    refuse({ reason: 'unknown_space', setting: 'odt', name: 'Rec.709' });
    await choose('odt', 'sRGB');
    fireEvent.click(field('gamut_compress')!);
    await save();
    expect(screen.getByRole('alert').textContent).toBe('That color space or transform is not one that the engine knows.');
    expect(dialog()).not.toBeNull();
    expect(field('odt')!.value).toBe('sRGB');
    expect(project().color?.odt).toBe('Rec.709');
    refuse({ reason: 'out_of_range', setting: 'graphics_white' });
    await save();
    expect(screen.getByRole('alert').textContent).toBe('A setting is outside its allowed range.');
    // Accepted, the same draft goes as it was, and the window closes.
    refuse(null);
    await save();
    expect(settingsSent(bridge)).toEqual(Array(3).fill([{ odt: 'sRGB', gamut_compress: true }]));
    expect(project().color?.odt).toBe('sRGB');
    expect(dialog()).toBeNull();
  });

  it('says to wait while a Save is being answered', async () => {
    const { bridge, hold } = await mount();
    await open();
    bridge.call.mockClear();
    let answer = () => {};
    hold(new Promise<void>((resolve) => { answer = resolve; }));
    await choose('science', 'acescc');
    await save();
    await save();
    expect(screen.getByRole('alert').textContent).toBe('The edit before is still being applied; try again in a moment.');
    answer();
    await settle();
    expect(sent(bridge)).toHaveLength(1);
  });

  it('shows a space as a gamut and a gamma where they are used apart', async () => {
    const { bridge } = await mount(projectOf({ color: custom({ separate_gamma: true, input: 'Sony S-Gamut3/S-Log3' }) }));
    await open();
    expect(field('input')!.value).toBe('Sony S-Gamut3');
    expect(field('input-gamma')!.value).toBe('S-Log3');
    expect([...field('timeline-gamma')!.options].map(each => each.value)).toEqual([...MEDIA_GAMMAS]);
    bridge.call.mockClear();
    await choose('input', 'Canon Cinema Gamut');
    await choose('timeline-gamma', 'Linear');
    await choose('output', 'bypass');
    expect(field('output-gamma')!.disabled).toBe(true);
    await save();
    expect(settingsSent(bridge)).toEqual([[{ input: 'Canon Cinema Gamut/Canon Log 2', timeline: 'Rec.709 Linear', output: 'bypass' }]]);
  });

  it('sends the ODT with a version of ACES that lacks the one chosen', async () => {
    const { bridge } = await mount(projectOf({ color: settings({ science: 'acescct', aces_version: '1.3', odt: 'P3-D60' }) }));
    await open();
    expect([...field('aces_version')!.options].map(each => each.textContent)).toEqual(['ACES 1.3', 'ACES 2.0']);
    expect(field('odt')!.value).toBe('P3-D60');
    bridge.call.mockClear();
    await choose('aces_version', '2.0');
    expect(field('odt')!.value).toBe('Rec.709');
    await save(true);
    await choose('odt', 'Rec.2020 ST2084 (1000 nits)');
    await save();
    expect(settingsSent(bridge)).toEqual([[{ aces_version: '2.0', odt: 'Rec.709' }], [{ odt: 'Rec.2020 ST2084 (1000 nits)' }]]);
  });

  it('sets the LUTs of the project by their names, the 1D and the 3D of each apart', async () => {
    const { bridge } = await mount(projectOf({ color: settings({ viewer_lut: { '1d': 'gone.cube', '3d': null } }) }));
    await open();
    const output = field('output_lut-3d')!;
    expect([...output.options].map(each => each.textContent)).toEqual(['No LUT selected', 'Film/Fuji 3513', 'Film/Kodak 2383', 'Looks/Teal', 'shaper']);
    // A LUT that the library does not have is still shown, as missing.
    expect(field('viewer_lut-1d')!.value).toBe('gone.cube');
    expect([...field('viewer_lut-1d')!.options].at(-1)!.textContent).toBe('gone.cube (missing)');
    bridge.call.mockClear();
    await choose('output_lut-3d', 'Film/Kodak 2383.cube');
    await choose('viewer_lut-1d', '');
    await choose('lut_interpolation', 'tetrahedral');
    // Update Lists reads the folders at once; it is no setting.
    fireEvent.click(screen.getByRole('button', { name: 'Update Lists' }));
    await settle();
    expect(calls(bridge, 'luts.list')).toHaveLength(1);
    await save();
    expect(settingsSent(bridge)).toEqual([[{ output_lut: { '3d': 'Film/Kodak 2383.cube' }, viewer_lut: { '1d': null }, lut_interpolation: 'tetrahedral' }]]);
  });

  it('offers 1D LUTs in the 1D menus and 3D LUTs in the 3D menus, and one that does not parse as unusable', async () => {
    await mount(projectOf(), { luts: mixedLibrary() });
    await open();
    const options = (name: string) => [...field(name)!.options].map(each => [each.textContent, each.disabled]);
    expect(options('input_lut-1d')).toEqual([['No LUT selected', false], ['Film/Linear', false], ['Log/Log to Lin', false]]);
    expect(options('input_lut-3d')).toEqual([
      ['No LUT selected', false], ['Film/Broken (cannot be read)', true], ['Film/Fuji 3513', false], ['Film/Kodak 2383', false],
      ['Looks/Teal', false], ['shaper', false],
    ]);
  });

  it('opens a folder of LUTs from its Open LUT Folder button, and only that folder', async () => {
    const luts = { ...library(), roots: [...library().roots, { place: 'library' as const, path: 'D:\\luts', exists: false }] };
    const { bridge } = await mount(projectOf(), { luts });
    await open();
    const buttons = screen.getAllByRole('button', { name: 'Open LUT Folder' });
    expect(buttons).toHaveLength(3);
    // A folder that is not there yet cannot be opened.
    expect(buttons.map(each => each.hasAttribute('disabled'))).toEqual([false, false, true]);
    expect(buttons[2].title).toBe('This folder does not exist yet.');
    fireEvent.click(buttons[1]);
    await settle();
    expect(bridge.openLutFolder).toHaveBeenCalledWith('C:\\Users\\me\\.columbina\\luts');
    expect(screen.queryByRole('alert')).toBeNull();
    // The shell would not open it: said at the top.
    bridge.openLutFolder.mockResolvedValueOnce(false);
    fireEvent.click(buttons[0]);
    await settle();
    expect(bridge.openLutFolder).toHaveBeenLastCalledWith('F:\\film\\.columbina\\studio');
    expect(screen.getByRole('alert').textContent).toBe('The folder could not be opened.');
    expect(sent(bridge)).toEqual([]);
  });

  it('keeps Use S-curve for contrast under General Options, and speaks Chinese', async () => {
    const { bridge } = await mount(projectOf(), { words: zhCN });
    fireEvent.click(screen.getByRole('button', { name: '项目设置' }));
    await settle();
    fireEvent.click(screen.getByRole('button', { name: '常规选项' }));
    bridge.call.mockClear();
    fireEvent.click(field('s_curve_contrast')!);
    await settle();
    expect(sent(bridge)).toEqual([]);
    expect(screen.getByText('对比度使用 S 曲线')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: '保存' }));
    await settle();
    expect(settingsSent(bridge)).toEqual([[{ s_curve_contrast: false }]]);
  });

  it('is not offered outside Windows', async () => {
    (window as { columbina?: unknown }).columbina = { platform: 'darwin' };
    await mount();
    expect(screen.queryByRole('button', { name: 'Project Settings' })).toBeNull();
  });
});

describe('Display HDR On Viewers If Available', () => {
  it('is sent to each engine as it is opened, also when it is started again, as the user chose it on this computer', async () => {
    localStorage.setItem('columbina-studio-viewer-hdr', 'false');
    const { bridge, restart } = await mount();
    expect(calls(bridge, 'view.display')).toEqual([{ hdr: false }]);
    await restart();
    await settle();
    expect(calls(bridge, 'view.display')).toEqual([{ hdr: false }, { hdr: false }]);
    // Turned on in the Project Settings: sent, and kept for the next time, when Save is clicked.
    await open();
    const box = field('display_hdr')!;
    expect(box.checked).toBe(false);
    fireEvent.click(box);
    await settle();
    expect(box.checked).toBe(true);
    expect(calls(bridge, 'view.display')).toHaveLength(2);
    expect(localStorage.getItem('columbina-studio-viewer-hdr')).toBe('false');
    // The engine said that Windows has HDR off for the monitors: the panel says why the viewer shows SDR.
    expect(screen.getByText(/Windows has HDR off for the display of the viewer/)).toBeDefined();
    await save();
    // Nothing of the project changed: no transaction, only the choice of this computer.
    expect(sent(bridge)).toEqual([]);
    expect(calls(bridge, 'view.display').at(-1)).toEqual({ hdr: true });
    expect(localStorage.getItem('columbina-studio-viewer-hdr')).toBe('true');
    await restart();
    await settle();
    expect(calls(bridge, 'view.display').at(-1)).toEqual({ hdr: true });
  });

  it('waits for the rest of the settings to be accepted', async () => {
    const { bridge, refuse } = await mount();
    await open();
    await choose('science', 'acescct');
    fireEvent.click(field('display_hdr')!);
    refuse({ reason: 'color_settings' });
    await save();
    expect(calls(bridge, 'view.display')).toEqual([{ hdr: true }]);
    expect(localStorage.getItem('columbina-studio-viewer-hdr')).toBeNull();
    refuse(null);
    await save();
    expect(calls(bridge, 'view.display').at(-1)).toEqual({ hdr: false });
  });

  it('is in the options menu of the viewer of the Color page, with the Viewer LUT of the project', async () => {
    const { bridge, project } = await mount(projectOf(), { page: 'Color' });
    expect(calls(bridge, 'view.display')).toEqual([{ hdr: true }]);
    fireEvent.click(screen.getByRole('button', { name: 'Viewer options' }));
    const menu = screen.getByRole('menu', { name: 'Viewer options' });
    const hdr = within(menu).getByRole('menuitem', { name: 'Display HDR On Viewers If Available' });
    expect(hdr.dataset.checked).toBe('true');
    fireEvent.click(hdr);
    await settle();
    expect(calls(bridge, 'view.display').at(-1)).toEqual({ hdr: false });
    expect(localStorage.getItem('columbina-studio-viewer-hdr')).toBe('false');
    // The menu lies over the viewer: the engine's view is hidden while it is open.
    fireEvent.click(screen.getByRole('button', { name: 'Viewer options' }));
    expect(document.body.dataset.overlay).toBe('1');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Viewer LUT' }));
    fireEvent.click(within(screen.getByRole('menu', { name: 'Viewer LUT' })).getByRole('menuitem', { name: 'Film' }));
    bridge.call.mockClear();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Kodak 2383' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_color_settings', settings: { viewer_lut: { '3d': 'Film/Kodak 2383.cube' } } }]]);
    expect(project().color?.viewer_lut).toEqual({ '1d': null, '3d': 'Film/Kodak 2383.cube' });
    expect(document.body.dataset.overlay).toBeUndefined();
    fireEvent.click(screen.getByRole('button', { name: 'Viewer options' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Viewer LUT' }));
    expect(within(screen.getByRole('menu', { name: 'Viewer LUT' })).getByRole('menuitem', { name: 'None' }).dataset.checked).toBeUndefined();
  });
});
