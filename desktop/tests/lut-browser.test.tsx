// @vitest-environment jsdom
/**
 * The LUTs of the Color page (Reference Manual 20, pp. 2970–2971, 3409–3413; the interaction
 * reference, §5): the LUT browser, its folders, favorites, search, menus, Refresh and Missing LUTs;
 * a LUT dragged onto a node or chosen in the node's LUT ▸ submenu, sent as `set_node` of its `lut`;
 * and Reveal Selected LUT.
 */
import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LutLibrary, Target } from '../src/studio/engine';
import { layoutGrade } from '../src/studio/color/graph';
import {
  draggedLut, favoriteLuts, lutBroken, lutDimension, lutFolders, lutMenuItems, lutsIn, lutTitle, searched, setNodeLut, usersOf, LUT_DRAG,
} from '../src/studio/color/luts';
import type { MenuItem } from '../src/studio/color/ColorMenu';
import { forgetFavorites } from '../src/studio/color/useLuts';
import { calls, corrector, library, lutGrade, mixedLibrary, mount, projectOf, sent, settle } from './colorEngine';
import { clip, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser();
  localStorage.clear();
  forgetFavorites();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('DragEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  // Every part of the page lies at the top left, so a point of the graph is a point of the window at zoom 1.
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('color-page')) return rect(0, 0, 1600, 900);
    return rect(0, 0, 600, 300);
  });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbina?: unknown }).columbina; localStorage.clear(); forgetFavorites(); });

const C1: Target = { level: 'clip', timeline: 't1', clip: 'c1' };
const words = { none: 'None', favorites: 'Favorites', broken: 'cannot be read' };

describe('the LUTs, worked out', () => {
  it('are laid out by folder, the project first, each folder under the one it is in', () => {
    const luts: LutLibrary = { ...library(), luts: [...library().luts, { name: 'Film/Print/2383 D65.cube', root: 1, size: 1, modified_us: 1, shadowed: false }] };
    expect(lutFolders(luts).map(each => [each.id, each.depth, each.count])).toEqual([
      ['project:', 0, 1], ['project:Looks', 1, 1], ['library:', 0, 1], ['library:Film', 1, 2], ['library:Film/Print', 2, 1],
    ]);
    expect(lutsIn(luts, 'library:Film').map(each => each.name)).toEqual(['Film/Fuji 3513.cube', 'Film/Kodak 2383.cube']);
    // The library's shaper is shadowed by the project's: it is shown where it lies, and named by no menu.
    expect(lutsIn(luts, 'library:').map(each => [each.name, each.shadowed])).toEqual([['shaper.cube', true]]);
    expect(lutTitle('Film/Print/2383 D65.CUBE')).toBe('2383 D65');
    expect(searched(lutsIn(luts, 'library:Film'), 'kod').map(each => each.name)).toEqual(['Film/Kodak 2383.cube']);
    expect(favoriteLuts(luts, new Set(['shaper.cube', 'gone.cube'])).map(each => [each.name, each.root])).toEqual([['shaper.cube', 0]]);
  });

  it('make a LUT ▸ submenu: None, the 1D LUTs and the 3D LUTs apart, each of the root then by folder, and Favorites', () => {
    const pick = vi.fn();
    const shape = (items: MenuItem[]): unknown[] => items.map(each => (each.items ? [each.label, shape(each.items)] : each.label));
    const items = lutMenuItems(mixedLibrary(), new Set(['Film/Kodak 2383.cube', 'Log/Log to Lin.cube']), 'Film/Kodak 2383.cube', pick, words);
    expect(shape(items)).toEqual([
      'None',
      ['1D LUT', [['Film', ['Linear']], ['Log', ['Log to Lin']]]],
      ['3D LUT', ['shaper', ['Film', ['Broken (cannot be read)', 'Fuji 3513', 'Kodak 2383']], ['Looks', ['Teal']]]],
      ['Favorites', ['Kodak 2383', 'Log to Lin']],
    ]);
    // The LUT chosen is checked; one that does not parse cannot be chosen.
    const film = items[2].items![1].items!;
    expect(film.map(each => [each.checked, each.disabled ?? false])).toEqual([[false, true], [false, false], [true, false]]);
    items[0].run!();
    items[1].items![1].items![0].run!();
    expect(pick.mock.calls).toEqual([[null], ['Log/Log to Lin.cube']]);
    // A menu of one kind, as the Viewer LUT's 3D: its LUTs, and its favorites.
    expect(shape(lutMenuItems(mixedLibrary(), new Set(['Film/Kodak 2383.cube', 'Log/Log to Lin.cube']), null, pick, words, 'viewer', '3d'))).toEqual([
      'None', 'shaper', ['Film', ['Broken (cannot be read)', 'Fuji 3513', 'Kodak 2383']], ['Looks', ['Teal']], ['Favorites', ['Kodak 2383']],
    ]);
    expect(lutMenuItems(null, new Set(), null, pick, words).map(each => [each.label, each.disabled ?? false]))
      .toEqual([['None', false], ['1D LUT', true], ['3D LUT', true], ['Favorites', true]]);
    // An engine from before the kinds: every LUT is taken as 3D, and none as broken.
    const old: LutLibrary = { ...library(), luts: library().luts.map(({ kind: _kind, status: _status, ...rest }) => rest) };
    expect(lutDimension(old.luts[0])).toBe('3d');
    expect(lutBroken(old.luts[0])).toBe(false);
  });

  it('put a LUT on a corrector, and nothing on another node, or the LUT it has', () => {
    const grade = lutGrade('shaper.cube');
    expect(setNodeLut(C1, grade.nodes[1], 'Looks/Teal.cube')).toEqual([{ op: 'set_node', grade: C1, id: 'n2', params: { lut: 'Looks/Teal.cube' } }]);
    expect(setNodeLut(C1, grade.nodes[0], null)).toEqual([{ op: 'set_node', grade: C1, id: 'n1', params: { lut: null } }]);
    expect(setNodeLut(C1, grade.nodes[0], 'shaper.cube')).toBeNull();
    expect(setNodeLut(C1, grade.nodes[2], 'shaper.cube')).toEqual({ refused: 'wrong_node' });
  });

  it('find the clips that name a LUT, by their own grade or their group\'s, and the timeline', () => {
    const line = { ...timeline([clip('c1', 0, 100, { grade: lutGrade('gone.cube') }), clip('c2', 100, 100, { group: 'g1' }), clip('c3', 200)]),
      grade: { nodes: [corrector('t', 'gone.cube')], links: [] } };
    const groups = [{ id: 'g1', name: 'Day', pre: { nodes: [corrector('p', 'gone.cube')], links: [] }, post: { nodes: [], links: [] } }];
    expect(usersOf('gone.cube', line, groups, ['c3', 'c2', 'c1'])).toEqual({ clips: ['c2', 'c1'], timeline: true });
    expect(usersOf('other.cube', line, groups, ['c1'])).toEqual({ clips: [], timeline: false });
  });

  it('carry a LUT in a drag of this session only', () => {
    const data = { getData: (type: string) => (type === LUT_DRAG ? JSON.stringify({ session: 2, name: 'a.cube' }) : '') } as unknown as DataTransfer;
    expect(draggedLut(data, 2)).toBe('a.cube');
    expect(draggedLut(data, 3)).toBeNull();
  });
});

/** What a drag carries, as the page sets and reads it. */
const transfer = () => {
  const data = new Map<string, string>();
  return { setData: (type: string, value: string) => { data.set(type, value); }, getData: (type: string) => data.get(type) ?? '',
    get types() { return [...data.keys()]; }, effectAllowed: 'none', dropEffect: 'none' };
};
const openBrowser = async () => { fireEvent.click(screen.getByRole('button', { name: 'LUTs' })); await settle(); };
const tile = (name: string) => document.querySelector<HTMLElement>(`[data-test="lut"][data-lut="${name}"]`)!;
const tiles = () => [...document.querySelectorAll<HTMLElement>('[data-test="lut"]')].map(each => each.dataset.lut);
const folder = (id: string) => document.querySelector<HTMLElement>(`[data-folder="${id}"]`)!;
const centerOf = (id: string) => { const placed = layoutGrade(lutGrade()).nodes.find(each => each.node.id === id)!; return { clientX: placed.x + 46, clientY: placed.y + 29 }; };
const canvas = () => document.querySelector<HTMLElement>('[data-test="node-canvas"]')!;

describe('the LUT browser', () => {
  it('lists the folders of LUTs and the LUTs of the one chosen, by file name, with a search', async () => {
    const { bridge } = await mount(projectOf(), { page: 'Color' });
    await openBrowser();
    const browser = screen.getByRole('region', { name: 'LUT browser' });
    expect(within(within(browser).getByRole('navigation', { name: 'LUT folders' })).getAllByRole('button').map(each => each.textContent))
      .toEqual(['Favorites', 'Project LUTs', 'Looks', 'LUT library', 'Film']);
    // The first folder that holds LUTs is shown at first.
    expect(tiles()).toEqual(['shaper.cube']);
    fireEvent.click(folder('library:Film'));
    expect(tiles()).toEqual(['Film/Fuji 3513.cube', 'Film/Kodak 2383.cube']);
    expect(tile('Film/Kodak 2383.cube').textContent).toContain('Kodak 2383');
    fireEvent.click(screen.getByRole('button', { name: 'Search' }));
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search' }), { target: { value: 'fuji' } });
    expect(tiles()).toEqual(['Film/Fuji 3513.cube']);
    // The shadowed copy of the library is shown, dimmed, and cannot be dragged or applied.
    fireEvent.click(screen.getByRole('button', { name: 'Search' }));
    fireEvent.click(folder('library:'));
    expect(tile('shaper.cube').dataset.shadowed).toBe('true');
    expect(tile('shaper.cube').draggable).toBe(false);
    expect(calls(bridge, 'luts.list')).toHaveLength(1);
  });

  it('keeps favorites on this computer, by the star or the menu', async () => {
    await mount(projectOf(), { page: 'Color' });
    await openBrowser();
    fireEvent.click(folder('library:Film'));
    fireEvent.click(within(tile('Film/Kodak 2383.cube')).getByRole('button', { name: 'Add to Favorites' }));
    fireEvent.contextMenu(tile('Film/Fuji 3513.cube'), { clientX: 50, clientY: 50 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add to Favorites' }));
    expect(JSON.parse(localStorage.getItem('columbina-studio-lut-favorites')!)).toEqual(['Film/Fuji 3513.cube', 'Film/Kodak 2383.cube']);
    fireEvent.click(screen.getByRole('button', { name: 'Favorites' }));
    expect(tiles()).toEqual(['Film/Fuji 3513.cube', 'Film/Kodak 2383.cube']);
    fireEvent.click(within(tile('Film/Fuji 3513.cube')).getByRole('button', { name: 'Remove from Favorites' }));
    expect(tiles()).toEqual(['Film/Kodak 2383.cube']);
    // Kept for the next window.
    forgetFavorites();
    cleanup();
    await mount(projectOf(), { page: 'Color' });
    await openBrowser();
    fireEvent.click(screen.getByRole('button', { name: 'Favorites' }));
    expect(tiles()).toEqual(['Film/Kodak 2383.cube']);
  });

  it('applies a LUT to the current node from its menu, and refreshes from the menu of the space around them', async () => {
    const { bridge } = await mount(projectOf(), { page: 'Color' });
    await openBrowser();
    fireEvent.click(folder('project:Looks'));
    bridge.call.mockClear();
    fireEvent.contextMenu(tile('Looks/Teal.cube'), { clientX: 50, clientY: 50 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Apply LUT to Current Node' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_node', grade: C1, id: 'n1', params: { lut: 'Looks/Teal.cube' } }]]);
    fireEvent.contextMenu(document.querySelector('[data-test="lut-list-area"]')!, { clientX: 50, clientY: 50 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Refresh' }));
    await settle();
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await settle();
    expect(calls(bridge, 'luts.list')).toHaveLength(2);
  });

  it('shows Missing LUTs where a grade names one the engine could not read, with the clips that name it', async () => {
    const { notify } = await mount(projectOf({}, lutGrade('gone.cube')), { page: 'Color' });
    await openBrowser();
    expect(screen.queryByRole('button', { name: 'Missing LUTs' })).toBeNull();
    await notify({ missing_luts: [{ name: 'gone.cube', message: 'there is no LUT gone.cube in the folder of the project or in the library' }] });
    fireEvent.click(screen.getByRole('button', { name: 'Missing LUTs' }));
    const rows = [...document.querySelectorAll<HTMLElement>('[data-test="missing-lut"]')];
    expect(rows.map(row => [...row.querySelectorAll('td')].map(cell => cell.textContent))).toEqual([['gone.cube', '01']]);
    expect(rows[0].title).toContain('there is no LUT gone.cube');
    await notify({ missing_luts: undefined });
    expect(screen.queryByRole('button', { name: 'Missing LUTs' })).toBeNull();
  });

  it('marks each LUT 1D or 3D, and one that does not parse as unusable, with why', async () => {
    const { bridge } = await mount(projectOf(), { page: 'Color', luts: mixedLibrary() });
    await openBrowser();
    fireEvent.click(folder('library:Film'));
    expect(tiles()).toEqual(['Film/Broken.cube', 'Film/Fuji 3513.cube', 'Film/Kodak 2383.cube', 'Film/Linear.cube']);
    const kinds = () => [...document.querySelectorAll<HTMLElement>('[data-test="lut-kind"]')].map(each => each.textContent);
    expect(kinds()).toEqual(['3D', '3D', '3D', '1D']);
    const broken = tile('Film/Broken.cube');
    expect(broken.dataset.broken).toBe('true');
    expect(broken.draggable).toBe(false);
    expect(broken.title).toBe('Film/Broken.cube\nThis LUT cannot be read, so it cannot be used.\nline 3: the file ends after 1 of the 8 rows of its tables');
    expect(tile('Film/Linear.cube').draggable).toBe(true);
    bridge.call.mockClear();
    fireEvent.contextMenu(broken, { clientX: 50, clientY: 50 });
    const apply = screen.getByRole('menuitem', { name: 'Apply LUT to Current Node' }) as HTMLButtonElement;
    expect(apply.disabled).toBe(true);
    fireEvent.click(apply);
    await settle();
    expect(sent(bridge)).toEqual([]);
  });

  it('says where the engine cannot list the LUTs', async () => {
    const { bridge } = await mount(projectOf(), { page: 'Color' });
    bridge.call.mockImplementation(async (method: string) => (method === 'luts.list'
      ? { ok: false, error: { code: -32601, message: 'method not found', data: null } } : { ok: true, result: {} }));
    await openBrowser();
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await settle();
    expect(screen.getByRole('alert').textContent).toContain('The LUTs could not be listed.');
  });
});

describe('a LUT on a node', () => {
  it('goes on a corrector it is dropped on, and not on a mixer or the space around', async () => {
    const { bridge } = await mount(projectOf(), { page: 'Color' });
    await openBrowser();
    fireEvent.click(folder('library:Film'));
    const dataTransfer = transfer();
    fireEvent.dragStart(tile('Film/Kodak 2383.cube'), { dataTransfer });
    expect(dataTransfer.effectAllowed).toBe('copy');
    bridge.call.mockClear();
    fireEvent.dragOver(canvas(), { ...centerOf('n3'), dataTransfer });
    expect(dataTransfer.dropEffect).toBe('none');
    fireEvent.drop(canvas(), { ...centerOf('n3'), dataTransfer });
    fireEvent.dragOver(canvas(), { clientX: 590, clientY: 290, dataTransfer });
    expect(dataTransfer.dropEffect).toBe('none');
    fireEvent.dragOver(canvas(), { ...centerOf('n2'), dataTransfer });
    expect(dataTransfer.dropEffect).toBe('copy');
    expect(document.querySelector('[data-node="n2"]')!.getAttribute('data-target')).toBe('true');
    fireEvent.drop(canvas(), { ...centerOf('n2'), dataTransfer });
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_node', grade: C1, id: 'n2', params: { lut: 'Film/Kodak 2383.cube' } }]]);
    // In place of the one it has (p. 3412).
    fireEvent.dragStart(tile('Film/Fuji 3513.cube'), { dataTransfer });
    fireEvent.dragOver(canvas(), { ...centerOf('n2'), dataTransfer });
    fireEvent.drop(canvas(), { ...centerOf('n2'), dataTransfer });
    await settle();
    expect(sent(bridge).at(-1)).toEqual([{ op: 'set_node', grade: C1, id: 'n2', params: { lut: 'Film/Fuji 3513.cube' } }]);
  });

  it('is chosen in the LUT ▸ submenu of a corrector, taken off with None, and revealed in the browser', async () => {
    const { bridge } = await mount(projectOf({}, lutGrade('Film/Kodak 2383.cube')), { page: 'Color' });
    bridge.call.mockClear();
    const nodeMenu = () => { fireEvent.contextMenu(document.querySelector('[data-node="n1"]')!, { clientX: 200, clientY: 60 }); fireEvent.click(screen.getByRole('menuitem', { name: 'LUT' })); };
    nodeMenu();
    const lutMenu = screen.getByRole('menu', { name: 'LUT' });
    expect(within(lutMenu).getAllByRole('menuitem').map(each => each.getAttribute('aria-label'))).toEqual(['None', '1D LUT', '3D LUT', 'Favorites']);
    // The library has no 1D LUT: its submenu is greyed.
    expect((within(lutMenu).getByRole('menuitem', { name: '1D LUT' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(within(lutMenu).getByRole('menuitem', { name: '3D LUT' }));
    const cubes = screen.getByRole('menu', { name: '3D LUT' });
    expect(within(cubes).getAllByRole('menuitem').map(each => each.getAttribute('aria-label'))).toEqual(['shaper', 'Film', 'Looks']);
    fireEvent.click(within(cubes).getByRole('menuitem', { name: 'Looks' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Teal' }));
    await settle();
    nodeMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'None' }));
    await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'set_node', grade: C1, id: 'n1', params: { lut: 'Looks/Teal.cube' } }],
      [{ op: 'set_node', grade: C1, id: 'n1', params: { lut: null } }],
    ]);
    // A mixer has no LUT.
    fireEvent.contextMenu(document.querySelector('[data-node="n3"]')!, { clientX: 200, clientY: 60 });
    expect(screen.queryByRole('menuitem', { name: 'LUT' })).toBeNull();
    fireEvent.keyDown(screen.getByRole('menu', { name: 'Node menu' }), { key: 'Escape' });
    // Reveal Selected LUT: greyed on a node without one; on one with, the browser opens on it.
    fireEvent.contextMenu(document.querySelector('[data-node="n1"]')!, { clientX: 200, clientY: 60 });
    expect((screen.getByRole('menuitem', { name: 'Reveal Selected LUT' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.keyDown(screen.getByRole('menu', { name: 'Node menu' }), { key: 'Escape' });
    cleanup();
    await mount(projectOf({}, lutGrade('Film/Kodak 2383.cube')), { page: 'Color' });
    expect(screen.queryByRole('region', { name: 'LUT browser' })).toBeNull();
    fireEvent.contextMenu(document.querySelector('[data-node="n1"]')!, { clientX: 200, clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Reveal Selected LUT' }));
    await settle();
    expect(screen.getByRole('region', { name: 'LUT browser' })).toBeDefined();
    expect(folder('library:Film').getAttribute('aria-pressed')).toBe('true');
    expect(tile('Film/Kodak 2383.cube').getAttribute('aria-selected')).toBe('true');
  });

  it('is said to be for a corrector where the current node is a mixer', async () => {
    const { bridge } = await mount(projectOf(), { page: 'Color' });
    fireEvent.pointerDown(document.querySelector('[data-node="n3"]')!, { button: 0, clientX: centerOf('n3').clientX, clientY: centerOf('n3').clientY });
    fireEvent.pointerUp(canvas(), { button: 0, clientX: centerOf('n3').clientX, clientY: centerOf('n3').clientY });
    await openBrowser();
    fireEvent.click(folder('project:Looks'));
    fireEvent.contextMenu(tile('Looks/Teal.cube'), { clientX: 50, clientY: 50 });
    expect((screen.getByRole('menuitem', { name: 'Apply LUT to Current Node' }) as HTMLButtonElement).disabled).toBe(true);
    expect(calls(bridge, 'transaction.apply')).toEqual([]);
  });
});
