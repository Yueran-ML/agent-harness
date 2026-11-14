// @vitest-environment jsdom
/**
 * The pictures of the LUT browser (Reference Manual 20, pp. 2970, 3259, 3410–3411; the interaction
 * reference, §5.1): each LUT's thumbnail on the engine's test image (`luts.preview`), and Live
 * Preview with Hover Scrub Preview: over a LUT, the viewer shows the current clip with it on the
 * current node (`viewer.preview`, which changes nothing of the project) and the thumbnail shows
 * the clip too, through its frames as the pointer moves across it where Hover Scrub says so.
 */
import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Target } from '../src/studio/engine';
import { forgetFavorites } from '../src/studio/color/useLuts';
import { Latest, readLutOptions, scrubbedFrame } from '../src/studio/color/lutPreview';
import { calls, lutGrade, mount, projectOf, sent, settle } from './colorEngine';
import { likeABrowser } from './mounted';

const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser();
  localStorage.clear();
  forgetFavorites();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  // A thumbnail is 100 pixels wide from the left of the window.
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('color-page')) return rect(0, 0, 1600, 900);
    if (this.classList.contains('color-lut-picture')) return rect(0, 0, 100, 56);
    return rect(0, 0, 600, 300);
  });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbina?: unknown }).columbina; localStorage.clear(); forgetFavorites(); });

const C1: Target = { level: 'clip', timeline: 't1', clip: 'c1' };
const openBrowser = async () => { fireEvent.click(screen.getByRole('button', { name: 'LUTs' })); await settle(); };
const tile = (name: string) => document.querySelector<HTMLElement>(`[data-test="lut"][data-lut="${name}"]`)!;
const picture = (name: string) => tile(name).querySelector<HTMLElement>('[data-test="lut-picture"]')!;
const shownPicture = (name: string) => tile(name).querySelector<HTMLImageElement>('[data-test="lut-thumbnail"]')?.getAttribute('src') ?? null;
const folder = (id: string) => document.querySelector<HTMLElement>(`[data-folder="${id}"]`)!;

/** The studio on the Color page, its engine drawing each LUT as `test:<name>` on the test image and as `clip:<name>@<frame>` on a frame. */
async function studio(project = projectOf()) {
  const mounted = await mount(project, { page: 'Color' });
  const plain = mounted.bridge.call.getMockImplementation()!;
  mounted.bridge.call.mockImplementation(async (method: string, params?: Record<string, unknown>) => {
    if (method === 'luts.preview') {
      const on = params!.frame as { frame: number } | undefined;
      return { ok: true, result: { previews: (params!.luts as string[]).map(lut => (lut === 'Film/Fuji 3513.cube'
        ? { lut, error: 'there is no LUT Film/Fuji 3513.cube' }
        : { lut, width: 16, height: 9, png: on ? `clip:${lut}@${on.frame}` : `test:${lut}` })) } };
    }
    if (method === 'viewer.preview') return { ok: true, result: { monitor: 'timeline', preview: params!.on === false ? null : params } };
    return plain(method, params);
  });
  return mounted;
}

/** Choose an option of the LUT browser's option menu, and a Hover Scrub Preview where one is named. */
async function choose(option: 'Live Preview' | 'Thumbnail and Viewer' | 'Thumbnail Only' | 'Off') {
  fireEvent.click(screen.getByRole('button', { name: 'LUT browser options' }));
  if (option !== 'Live Preview') fireEvent.click(screen.getByRole('menuitem', { name: 'Hover Scrub Preview' }));
  fireEvent.click(screen.getByRole('menuitem', { name: option }));
  await settle();
}

describe('the pictures of the LUT browser, worked out', () => {
  it('scrub the frames of the current clip across a thumbnail, and keep their options on this computer', () => {
    const clip = { start: 100, length: 50 };
    expect([0, 0.5, 0.999, 1, -1, 2].map(across => scrubbedFrame(clip, across))).toEqual([100, 125, 149, 149, 100, 149]);
    expect(scrubbedFrame({ start: 7, length: 0 }, 0.5)).toBe(7);
    expect(readLutOptions()).toEqual({ live: false, scrub: 'off' });
    localStorage.setItem('columbina-studio-lut-browser', JSON.stringify({ live: true, scrub: 'both' }));
    expect(readLutOptions()).toEqual({ live: true, scrub: 'both' });
    localStorage.setItem('columbina-studio-lut-browser', JSON.stringify({ live: 'yes', scrub: 'sideways' }));
    expect(readLutOptions()).toEqual({ live: false, scrub: 'off' });
    localStorage.setItem('columbina-studio-lut-browser', '{');
    expect(readLutOptions()).toEqual({ live: false, scrub: 'off' });
  });

  it('send one call at a time, and of those asked for meanwhile the latest', async () => {
    const answers: Array<() => void> = [];
    const seen: number[] = [];
    const latest = new Latest<number>(params => new Promise<void>((resolve) => { seen.push(params); answers.push(resolve); }));
    latest.push(1);
    latest.push(2);
    latest.push(3);
    expect(seen).toEqual([1]);
    answers.shift()!();
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(seen).toEqual([1, 3]);
    answers.shift()!();
    await Promise.resolve(); await Promise.resolve();
    expect(seen).toEqual([1, 3]);
  });
});

describe('the thumbnails of the LUT browser', () => {
  it('show each LUT on the test image, asked of the engine once, and a mark where it cannot draw one', async () => {
    const { bridge } = await studio();
    await openBrowser();
    expect(shownPicture('shaper.cube')).toBe('data:image/png;base64,test:shaper.cube');
    fireEvent.click(folder('library:Film'));
    await settle();
    expect(shownPicture('Film/Kodak 2383.cube')).toBe('data:image/png;base64,test:Film/Kodak 2383.cube');
    // One the engine cannot read shows the mark of a LUT.
    expect(shownPicture('Film/Fuji 3513.cube')).toBeNull();
    expect(tile('Film/Fuji 3513.cube').querySelector('svg')).not.toBeNull();
    // Back to a folder seen: not asked again.
    fireEvent.click(folder('project:'));
    await settle();
    expect(calls(bridge, 'luts.preview')).toEqual([
      { luts: ['shaper.cube'], height: 96 },
      { luts: ['Film/Fuji 3513.cube', 'Film/Kodak 2383.cube'], height: 96 },
    ]);
    expect(shownPicture('shaper.cube')).toBe('data:image/png;base64,test:shaper.cube');
  });
});

describe('Live Preview of the LUT browser', () => {
  it('is off at first: a LUT under the pointer changes nothing of the viewer', async () => {
    const { bridge } = await studio();
    await openBrowser();
    fireEvent.pointerEnter(picture('shaper.cube'), { clientX: 50 });
    await settle();
    expect(calls(bridge, 'viewer.preview')).toEqual([]);
    expect(screen.getByRole('button', { name: 'LUT browser options' }).dataset.live).toBeUndefined();
  });

  it('shows the LUT under the pointer on the current node at the playhead, in the viewer and its thumbnail, and nothing once it leaves', async () => {
    const { bridge } = await studio();
    await openBrowser();
    await choose('Live Preview');
    expect(JSON.parse(localStorage.getItem('columbina-studio-lut-browser')!)).toEqual({ live: true, scrub: 'off' });
    expect(screen.getByRole('button', { name: 'LUT browser options' }).dataset.live).toBe('true');
    fireEvent.click(folder('library:Film'));
    await settle();
    fireEvent.pointerEnter(picture('Film/Kodak 2383.cube'), { clientX: 80 });
    await settle();
    expect(calls(bridge, 'viewer.preview')).toEqual([{ grade: C1, node: 'n1', lut: 'Film/Kodak 2383.cube' }]);
    // The thumbnail under the pointer is the current clip with it, at the playhead.
    expect(calls(bridge, 'luts.preview').at(-1)).toEqual({
      luts: ['Film/Kodak 2383.cube'], height: 96, frame: { timeline: 't1', frame: 0, clip: 'c1', grade: C1, node: 'n1' },
    });
    expect(shownPicture('Film/Kodak 2383.cube')).toBe('data:image/png;base64,clip:Film/Kodak 2383.cube@0');
    // Without Hover Scrub, a move across it changes nothing.
    fireEvent.pointerMove(picture('Film/Kodak 2383.cube'), { clientX: 20 });
    await settle();
    expect(calls(bridge, 'viewer.preview')).toHaveLength(1);
    // Away: the viewer shows the project again, and the thumbnail the test image. Nothing of the project was changed.
    fireEvent.pointerLeave(picture('Film/Kodak 2383.cube'));
    await settle();
    expect(calls(bridge, 'viewer.preview').at(-1)).toEqual({ on: false });
    expect(shownPicture('Film/Kodak 2383.cube')).toBe('data:image/png;base64,test:Film/Kodak 2383.cube');
    expect(sent(bridge)).toEqual([]);
    // One that no node may name (a copy that another root shadows) is not previewed.
    fireEvent.click(folder('library:'));
    await settle();
    fireEvent.pointerEnter(picture('shaper.cube'), { clientX: 80 });
    await settle();
    expect(calls(bridge, 'viewer.preview')).toHaveLength(2);
  });

  it('scrubs the current clip across the thumbnail, in the viewer too or the thumbnail alone, as Hover Scrub Preview says', async () => {
    const { bridge } = await studio();
    await openBrowser();
    await choose('Live Preview');
    await choose('Thumbnail and Viewer');
    expect(JSON.parse(localStorage.getItem('columbina-studio-lut-browser')!)).toEqual({ live: true, scrub: 'both' });
    // The first clip is frames 0–99: a quarter of the way across is frame 25, three quarters 75.
    fireEvent.pointerEnter(picture('shaper.cube'), { clientX: 25 });
    await settle();
    fireEvent.pointerMove(picture('shaper.cube'), { clientX: 75 });
    await settle();
    expect(calls(bridge, 'viewer.preview')).toEqual([
      { grade: C1, node: 'n1', lut: 'shaper.cube', frame: 25 },
      { grade: C1, node: 'n1', lut: 'shaper.cube', frame: 75 },
    ]);
    expect(shownPicture('shaper.cube')).toBe('data:image/png;base64,clip:shaper.cube@75');
    // The thumbnail alone: the viewer at the playhead.
    await choose('Thumbnail Only');
    fireEvent.pointerMove(picture('shaper.cube'), { clientX: 50 });
    await settle();
    expect(calls(bridge, 'viewer.preview').at(-1)).toEqual({ grade: C1, node: 'n1', lut: 'shaper.cube' });
    expect(shownPicture('shaper.cube')).toBe('data:image/png;base64,clip:shaper.cube@50');
    // Live Preview turned off as the pointer is over a LUT: the viewer shows the project again.
    await choose('Live Preview');
    expect(calls(bridge, 'viewer.preview').at(-1)).toEqual({ on: false });
    expect(shownPicture('shaper.cube')).toBe('data:image/png;base64,test:shaper.cube');
  });

  it('ends as the browser is closed, and shows nothing where the current node is no corrector', async () => {
    const mixer = lutGrade();
    const { bridge } = await studio(projectOf({}, lutGrade('Looks/Teal.cube')));
    await openBrowser();
    await choose('Live Preview');
    fireEvent.pointerEnter(picture('shaper.cube'), { clientX: 50 });
    await settle();
    expect(calls(bridge, 'viewer.preview')).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: 'LUTs' }));
    await settle();
    expect(calls(bridge, 'viewer.preview').at(-1)).toEqual({ on: false });
    cleanup();
    const other = await studio(projectOf({}, {
      nodes: mixer.nodes.filter(node => node.kind !== 'corrector'),
      links: [{ from: 'source', to: 'n3', kind: 'rgb', input: 0 }, { from: 'n3', to: 'output', kind: 'rgb', input: 0 }],
    }));
    await openBrowser();
    fireEvent.pointerEnter(picture('shaper.cube'), { clientX: 50 });
    await settle();
    expect(calls(other.bridge, 'viewer.preview')).toEqual([]);
  });
});
