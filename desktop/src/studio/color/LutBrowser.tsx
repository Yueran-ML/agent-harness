import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type MouseEvent as ReactMouseEvent } from 'react';
import { Folder, Layers3, RotateCw, Search, Star, TriangleAlert } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import type { Group, LutFile, Timeline } from '../engine';
import { ColorMenu, type Menu } from './ColorMenu';
import {
  FAVORITES, favoriteLuts, folderId, folderOf, LUT_DRAG, lutFolders, lutsIn, lutTitle, MISSING, placeOf, searched, usersOf,
} from './luts';
import { toggleFavorite, useFavoriteLuts, useLutLibrary } from './useLuts';
import './luts.css';

/** A LUT the engine could not read, as the timeline state tells it (PROTOCOL.md, "Grades in the picture"). */
export interface MissingLut { name: string; message: string }

/** A LUT that Reveal Selected LUT asks the browser to show; `asked` tells one request from the next of the same LUT. */
export interface RevealedLut { name: string; asked: number }

/**
 * The LUT browser of the Color page (Reference Manual 20, pp. 2970–2971, 3410–3413; the interaction
 * reference, §5.1): at the left the folders of LUTs, the project's and the library's, with Favorites
 * at the top and Missing LUTs where a grade names a LUT the engine could not read; at the right the
 * LUTs of the folder chosen, by their file names (Thumbnail Sort's Filename), filtered by the search.
 *
 * A LUT is dragged onto a node of the node editor to put it there; its right-click menu applies it
 * to the current node or adds it to the favorites, as the star does that shows on it under the
 * pointer; a right-click elsewhere refreshes the list from the disk. The thumbnails of Resolve show
 * each LUT on a test image or on the frame shown: the engine cannot draw a LUT on a frame by itself
 * yet, so each shows its name.
 */
export function LutBrowser({ bridge, session, canApply, onApply, missing, timeline, groups, numbers, revealed }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>;
  session: number;
  /** Whether there is a current node that a LUT can go on (a corrector). */
  canApply: boolean;
  /** Apply LUT to Current Node. */
  onApply: (name: string) => void;
  missing: readonly MissingLut[];
  /** The timeline shown and the groups, which say which clips name a missing LUT, numbered as the thumbnail timeline numbers them. */
  timeline: Timeline | null;
  groups: readonly Group[];
  numbers: ReadonlyMap<string, number>;
  revealed: RevealedLut | null;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage;
  const lutWords = labels.lutPanel;
  const said = words.hint.studio.colorPage;
  const { library, error, loading, refresh } = useLutLibrary(bridge);
  const favorites = useFavoriteLuts();
  const folders = useMemo(() => (library === null ? [] : lutFolders(library)), [library]);
  const [category, setCategory] = useState<string | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [searching, setSearching] = useState(false);
  const [menu, setMenu] = useState<Menu | null>(null);
  const list = useRef<HTMLUListElement>(null);

  // The folder shown: the one chosen while it is there, else the first that holds LUTs, else Favorites.
  const shownCategory = category === MISSING && missing.length > 0 ? MISSING
    : category === FAVORITES || folders.some(each => each.id === category) ? category!
      : folders.find(each => each.count > 0)?.id ?? FAVORITES;
  const luts = useMemo(() => {
    if (library === null || shownCategory === MISSING) return [];
    return searched(shownCategory === FAVORITES ? favoriteLuts(library, favorites) : lutsIn(library, shownCategory), query);
  }, [library, shownCategory, favorites, query]);

  // Reveal Selected LUT: its folder, and it chosen in it (p. 3412).
  useEffect(() => {
    if (revealed === null || library === null) return;
    const lut = library.luts.find(each => each.name === revealed.name && !each.shadowed);
    if (!lut) { if (missing.some(each => each.name === revealed.name)) setCategory(MISSING); return; }
    setCategory(folderId(placeOf(library, lut), folderOf(lut.name)));
    setQuery('');
    setChosen(`${lut.root}:${lut.name}`);
  }, [revealed, library]);
  useEffect(() => {
    if (chosen === null) return;
    [...(list.current?.children ?? [])].find(each => (each as HTMLElement).dataset.key === chosen)?.scrollIntoView({ block: 'nearest' });
  }, [chosen, shownCategory]);

  const keyOf = (lut: LutFile) => `${lut.root}:${lut.name}`;
  const drag = useCallback((lut: LutFile, event: DragEvent) => {
    event.dataTransfer.setData(LUT_DRAG, JSON.stringify({ session, name: lut.name }));
    event.dataTransfer.effectAllowed = 'copy';
  }, [session]);
  const lutMenu = (lut: LutFile, event: ReactMouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    setChosen(keyOf(lut));
    const liked = favorites.has(lut.name);
    setMenu({ x: event.clientX, y: event.clientY, label: lutWords.menu, items: [
      { id: 'lut-apply', label: lutWords.apply, disabled: !canApply || lut.shadowed, run: () => onApply(lut.name) },
      { id: 'lut-favorite', label: liked ? lutWords.removeFavorite : lutWords.addFavorite, disabled: lut.shadowed, run: () => toggleFavorite(lut.name) },
    ] });
  };
  const blankMenu = (event: ReactMouseEvent) => {
    event.preventDefault();
    setMenu({ x: event.clientX, y: event.clientY, label: lutWords.menu, items: [{ id: 'lut-refresh', label: lutWords.refresh, run: refresh }] });
  };
  const closeMenu = useCallback(() => setMenu(null), []);

  const project = folders.filter(each => each.place === 'project');
  const kept = folders.filter(each => each.place === 'library');
  const folderButton = (each: typeof folders[number]) => <li key={each.id}>
    <button type="button" className="color-lut-folder" aria-pressed={shownCategory === each.id} style={{ paddingLeft: 6 + each.depth * 12 }}
      onClick={() => setCategory(each.id)} data-test="lut-folder" data-folder={each.id}>
      <Folder size={12} aria-hidden="true" />
      <span>{each.folder === '' ? (each.place === 'project' ? lutWords.project : lutWords.library) : each.folder.slice(each.folder.lastIndexOf('/') + 1)}</span>
    </button>
  </li>;

  return <div className="color-luts" data-test="lut-browser">
    <header className="color-luts-head">
      <h2 className="overline">{labels.lutBrowser}</h2>
      <span className="color-gap" />
      <button type="button" className="icon-btn" aria-pressed={searching} aria-label={lutWords.search} title={lutWords.search}
        onClick={() => { setSearching(open => !open); setQuery(''); }} data-test="lut-search-toggle"><Search size={13} /></button>
      <button type="button" className="icon-btn" aria-label={lutWords.refresh} title={lutWords.refresh} disabled={loading}
        onClick={refresh} data-test="lut-refresh"><RotateCw size={13} /></button>
    </header>
    {searching && <input className="field color-lut-search" type="search" aria-label={lutWords.search} placeholder={lutWords.search} autoFocus
      value={query} onChange={event => setQuery(event.target.value)}
      onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); setSearching(false); setQuery(''); } }} data-test="lut-search" />}
    {error !== null && <p className="caption color-lut-problem" role="alert" data-test="lut-error">{said.lutsFailed}<span lang="en"> {error}</span></p>}
    {library?.truncated && <p className="caption" role="status">{said.lutsTruncated}</p>}
    <div className="color-luts-body">
      <nav className="color-lut-folders" aria-label={lutWords.folders}>
        <ul>
          <li><button type="button" className="color-lut-folder" aria-pressed={shownCategory === FAVORITES} onClick={() => setCategory(FAVORITES)}
            data-test="lut-favorites"><Star size={12} aria-hidden="true" /><span>{lutWords.favorites}</span></button></li>
          {project.map(folderButton)}
          {kept.map(folderButton)}
          {missing.length > 0 && <li><button type="button" className="color-lut-folder" data-missing="" aria-pressed={shownCategory === MISSING}
            onClick={() => setCategory(MISSING)} data-test="lut-missing-tab"><TriangleAlert size={12} aria-hidden="true" /><span>{lutWords.missing}</span></button></li>}
        </ul>
      </nav>
      <div className="color-lut-list-area" onContextMenu={blankMenu} data-test="lut-list-area">
        {shownCategory === MISSING ? <MissingList missing={missing} timeline={timeline} groups={groups} numbers={numbers} />
          : luts.length === 0 ? <p className="caption color-empty" data-test="lut-none">
            {library === null ? (loading ? '' : said.lutsFailed) : shownCategory === FAVORITES && query === '' ? said.noFavorites
              : library.luts.length === 0 ? said.noLuts : said.noLutsHere}
          </p>
            : <ul ref={list} className="color-lut-list" aria-label={lutWords.list} data-test="lut-list">
              {luts.map((lut) => {
                const liked = favorites.has(lut.name);
                return <li key={keyOf(lut)} className="color-lut" data-key={keyOf(lut)} data-lut={lut.name} aria-selected={chosen === keyOf(lut)}
                  data-shadowed={lut.shadowed || undefined} title={lut.shadowed ? `${lut.name}\n${said.lutShadowed}` : lut.name}
                  draggable={!lut.shadowed} onDragStart={event => drag(lut, event)} onClick={() => setChosen(keyOf(lut))}
                  onContextMenu={event => lutMenu(lut, event)} data-test="lut">
                  <span className="color-lut-picture" aria-hidden="true"><Layers3 size={22} /></span>
                  <span className="color-lut-name">{lutTitle(lut.name)}</span>
                  {!lut.shadowed && <button type="button" className="color-lut-star" aria-pressed={liked} data-liked={liked || undefined}
                    aria-label={liked ? lutWords.removeFavorite : lutWords.addFavorite} title={liked ? lutWords.removeFavorite : lutWords.addFavorite}
                    onClick={(event) => { event.stopPropagation(); toggleFavorite(lut.name); }} data-test="lut-star">
                    <Star size={12} fill={liked ? 'currentColor' : 'none'} /></button>}
                </li>;
              })}
            </ul>}
      </div>
    </div>
    {menu && <ColorMenu menu={menu} onClose={closeMenu} />}
  </div>;
}

/** The Missing LUTs tab (p. 3413): each LUT the engine could not read, why, and the clips whose grades name it. */
function MissingList({ missing, timeline, groups, numbers }: {
  missing: readonly MissingLut[]; timeline: Timeline | null; groups: readonly Group[]; numbers: ReadonlyMap<string, number>;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage;
  const order = useMemo(() => [...numbers.entries()].sort((a, b) => a[1] - b[1]).map(([id]) => id), [numbers]);
  return <div className="color-lut-missing" data-test="lut-missing">
    <p className="caption">{words.hint.studio.colorPage.missingLuts}</p>
    <table>
      <thead><tr><th scope="col">{labels.lutPanel.name}</th><th scope="col">{labels.lutPanel.clips}</th></tr></thead>
      <tbody>
        {missing.map((each) => {
          const users = usersOf(each.name, timeline, groups, order);
          const used = [...users.clips.map(id => String(numbers.get(id) ?? '').padStart(2, '0')), ...(users.timeline ? [labels.levels.timeline] : [])];
          return <tr key={each.name} title={each.message} data-test="missing-lut">
            <td>{each.name}</td>
            <td>{used.join(', ')}</td>
          </tr>;
        })}
      </tbody>
    </table>
  </div>;
}
