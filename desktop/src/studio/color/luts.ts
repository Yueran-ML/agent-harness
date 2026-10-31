/**
 * The LUTs of the library as the Color page shows them (Reference Manual 20, pp. 2970–2971,
 * 3409–3413; the interaction reference, §5), worked out apart from drawing them: the folders of
 * the LUT browser's sidebar, the LUTs of each, the LUT ▸ submenus of a node and of the viewer, the
 * clips that name a LUT the engine could not read, and the edit that puts a LUT on a node.
 *
 * A node names a LUT by its path from the folder it is in (`luts.list`); the engine looks for that
 * name in the folder of the project, then in the library. So the browser shows where each one lies
 * (the project's LUTs, then the library's), and the menus, which only name them, show them by name.
 */
import type { Grade, GradeNode, Group, LutFile, LutLibrary, Target, Timeline } from '../engine';
import type { MenuItem } from './ColorMenu';
import type { Worked } from './nodeEdits';

/** The LUT ▸ submenu of a node (p. 3412), named as Resolve names it in every language. */
export const LUT_MENU = 'LUT';

/** What a LUT is called on the page: its file's name, without `.cube`. */
export function lutTitle(name: string): string {
  const file = name.slice(name.lastIndexOf('/') + 1);
  return file.replace(/\.cube$/i, '');
}

/** The folder a LUT is in, from its root ('' for the root itself). */
export function folderOf(name: string): string {
  const slash = name.lastIndexOf('/');
  return slash < 0 ? '' : name.slice(0, slash);
}

/** Where a LUT lies, as the sidebar sorts them: the folder of the project, or the library. */
export type LutPlace = 'project' | 'library';

/** A folder of the sidebar: under which place, its path ('' for the place itself), how deep it is, and how many LUTs it holds itself. */
export interface LutFolder { id: string; place: LutPlace; folder: string; depth: number; count: number }

export const folderId = (place: LutPlace, folder: string) => `${place}:${folder}`;
/** The id of the Favorites category and of the Missing LUTs one. */
export const FAVORITES = 'favorites';
export const MISSING = 'missing';

/** The place of a LUT of the library. */
export function placeOf(library: LutLibrary, lut: LutFile): LutPlace {
  return library.roots[lut.root]?.place === 'project' ? 'project' : 'library';
}

/**
 * The folders of the sidebar, as a tree in the order it is drawn: the project's LUTs, then the
 * library's (its folders, all of them as one, since the engine looks in each in turn), each
 * folder under the one it is in, in the order of the names. A place without any LUT is left out.
 */
export function lutFolders(library: LutLibrary): LutFolder[] {
  const folders: LutFolder[] = [];
  for (const place of ['project', 'library'] as const) {
    const luts = library.luts.filter(lut => placeOf(library, lut) === place);
    if (luts.length === 0) continue;
    const counts = new Map<string, number>([['', 0]]);
    for (const lut of luts) {
      const folder = folderOf(lut.name);
      counts.set(folder, (counts.get(folder) ?? 0) + 1);
      // Every folder above it is one of the tree, holding it or not.
      for (let at = folder.indexOf('/'); at >= 0; at = folder.indexOf('/', at + 1)) {
        const above = folder.slice(0, at);
        if (!counts.has(above)) counts.set(above, 0);
      }
    }
    const paths = [...counts.keys()].sort((a, b) => (a === '' ? -1 : b === '' ? 1 : a < b ? -1 : a > b ? 1 : 0));
    for (const folder of paths) {
      folders.push({ id: folderId(place, folder), place, folder, depth: folder === '' ? 0 : folder.split('/').length, count: counts.get(folder) ?? 0 });
    }
  }
  return folders;
}

/** The LUTs of a folder of the sidebar (not those of the folders in it), sorted by file name as Thumbnail Sort's Filename sorts them. */
export function lutsIn(library: LutLibrary, id: string): LutFile[] {
  return library.luts
    .filter(lut => folderId(placeOf(library, lut), folderOf(lut.name)) === id)
    .sort(byTitle);
}

const byTitle = (a: LutFile, b: LutFile) => lutTitle(a.name).localeCompare(lutTitle(b.name), undefined, { sensitivity: 'base' }) || a.root - b.root;

/** The LUTs that a node can name: one of each name, the one the engine reads (not shadowed). */
export function namedLuts(library: LutLibrary): LutFile[] {
  return library.luts.filter(lut => !lut.shadowed);
}

/** The LUTs of the favorites, each the one the engine reads by that name, sorted by file name. */
export function favoriteLuts(library: LutLibrary, favorites: ReadonlySet<string>): LutFile[] {
  return namedLuts(library).filter(lut => favorites.has(lut.name)).sort(byTitle);
}

/** The LUTs whose title has the words searched for, as the search of the browser filters the category shown (p. 2971). */
export function searched(luts: readonly LutFile[], words: string): LutFile[] {
  const wanted = words.trim().toLocaleLowerCase();
  return wanted === '' ? [...luts] : luts.filter(lut => lutTitle(lut.name).toLocaleLowerCase().includes(wanted));
}

/**
 * The items of a LUT ▸ submenu (p. 3412; the interaction reference, §5.2): None at the top (the
 * manual does not say how a LUT is taken off), then the LUTs of the root, then a submenu for each
 * folder with the LUTs under it, each by its path below that folder, and Favorites ▸. The LUT
 * chosen is checked. Folders are one deep, so that a submenu with submenus need not scroll.
 */
export function lutMenuItems(library: LutLibrary | null, favorites: ReadonlySet<string>, chosen: string | null,
  pick: (name: string | null) => void, words: { none: string; favorites: string }, id = 'lut'): MenuItem[] {
  const named = library === null ? [] : namedLuts(library).slice().sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
  const item = (lut: LutFile, label: string, prefix: string): MenuItem => ({
    id: `${id}-${prefix}${lut.name}`, label, checked: lut.name === chosen, run: () => pick(lut.name),
  });
  const items: MenuItem[] = [{ id: `${id}-none`, label: words.none, checked: chosen === null, run: () => pick(null) }];
  const folders = new Map<string, LutFile[]>();
  for (const lut of named) {
    const slash = lut.name.indexOf('/');
    if (slash < 0) { items.push(item(lut, lutTitle(lut.name), '')); continue; }
    const top = lut.name.slice(0, slash);
    folders.set(top, [...(folders.get(top) ?? []), lut]);
  }
  for (const [folder, luts] of folders) {
    items.push({
      id: `${id}-folder-${folder}`, label: folder,
      items: luts.map(lut => item(lut, lut.name.slice(folder.length + 1).replace(/\.cube$/i, ''), '')),
    });
  }
  const liked = library === null ? [] : favoriteLuts(library, favorites);
  items.push({
    id: `${id}-favorites`, label: words.favorites, disabled: liked.length === 0,
    items: liked.map(lut => item(lut, lutTitle(lut.name), 'favorite-')),
  });
  return items;
}

/**
 * The edit that puts a LUT on a node, or takes it off (null): `set_node` of its `lut`, one
 * transaction. Only a corrector has one (`wrong_node`); the LUT it has already sends nothing.
 */
export function setNodeLut(target: Target, node: GradeNode | undefined, name: string | null): Worked {
  if (node === undefined || node.kind !== 'corrector') return { refused: 'wrong_node' };
  if (node.params.lut === name) return null;
  return [{ op: 'set_node', grade: target, id: node.id, params: { lut: name } }];
}

/** Whether a grade has a corrector that names the LUT. */
const names = (grade: Grade, lut: string) => grade.nodes.some(node => node.kind === 'corrector' && node.params.lut === lut);

/**
 * The clips of a timeline whose grades name a LUT, as the Missing LUTs tab lists them (p. 3413):
 * by their clips' own grade or their group's, in the order of the thumbnail timeline (`order`);
 * and whether the timeline's own grade names it.
 */
export function usersOf(lut: string, timeline: Timeline | null, groups: readonly Group[], order: readonly string[]): { clips: string[]; timeline: boolean } {
  if (timeline === null) return { clips: [], timeline: false };
  const grouped = new Map(groups.map(group => [group.id, names(group.pre, lut) || names(group.post, lut)]));
  const clips = timeline.tracks.flatMap(track => (track.kind === 'subtitle' ? [] : track.clips))
    .filter(clip => names(clip.grade, lut) || (clip.group !== null && grouped.get(clip.group) === true))
    .map(clip => clip.id);
  return { clips: order.filter(id => clips.includes(id)), timeline: names(timeline.grade, lut) };
}

/** The type of what a LUT dragged from the browser carries: `{session, name}` as JSON. */
export const LUT_DRAG = 'application/x-columbina-lut';

/** The name of the LUT that a drag carries from the browser of this session, or null. */
export function draggedLut(data: DataTransfer | null, session: number): string | null {
  const said = data?.getData(LUT_DRAG);
  if (!said) return null;
  try {
    const parsed = JSON.parse(said) as { session?: unknown; name?: unknown };
    return parsed.session === session && typeof parsed.name === 'string' ? parsed.name : null;
  } catch {
    return null;
  }
}

/** Whether a drag carries a LUT, which can be told before it is dropped (its data cannot be read until then). */
export function carriesLut(data: DataTransfer | null): boolean {
  return [...(data?.types ?? [])].includes(LUT_DRAG);
}
