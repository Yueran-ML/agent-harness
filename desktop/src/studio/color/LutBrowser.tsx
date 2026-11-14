import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type MouseEvent as ReactMouseEvent } from 'react';
import { Ellipsis, Folder, Layers3, RotateCw, Search, Star, TriangleAlert } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import type { Group, LutFile, Timeline } from '../engine';
import { ColorMenu, type Menu } from './ColorMenu';
import {
  FAVORITES, favoriteLuts, folderId, folderOf, LUT_DRAG, lutBroken, lutDimension, lutFolders, lutsIn, lutTitle, lutUsable, MISSING, placeOf, searched,
  usersOf,
} from './luts';
import { toggleFavorite, useFavoriteLuts, useLutLibrary } from './useLuts';
import {
  HOVER_SCRUBS, keepLutOptions, readLutOptions, thumbnailKey, useLivePreview, useLutThumbnails, type LutBrowserOptions, type PreviewPlace,
} from './lutPreview';
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
 * pointer; a right-click elsewhere refreshes the list from the disk. Each LUT shows its thumbnail
 * on the test image ("By default, all LUTs appear with a test thumbnail", p. 3410), drawn by the
 * engine, and whether it is a 1D or a 3D LUT (the Type that Resolve's Column view and Thumbnail
 * Sort show, p. 3411). The option menu (…) has Live Preview (p. 3411): over a LUT, the viewer and
 * its thumbnail show the current clip with it on the current node; and Hover Scrub Preview (p.
 * 3259), how they go through the clip as the pointer moves across the thumbnail. A LUT that the
 * engine could not parse is shown marked, with why under the pointer, and cannot be dragged or
 * applied.
 */
export function LutBrowser({ bridge, session, native = false, canApply, onApply, missing, timeline, groups, numbers, revealed, place = null,
  interpolation = 'trilinear' }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>;
  session: number;
  /** Whether an engine draws: thumbnails and Live Preview are its. */
  native?: boolean;
  /** Where Live Preview shows a LUT: the current corrector, the current clip and the playhead; none where there is no corrector. */
  place?: PreviewPlace | null;
  /** How the project reads a cube between its points, which the thumbnails are drawn by. */
  interpolation?: string;
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
  const [options, setOptions] = useState<LutBrowserOptions>(readLutOptions);
  const choose = (change: Partial<LutBrowserOptions>) => setOptions((old) => { const next = { ...old, ...change }; keepLutOptions(next); return next; });

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
      { id: 'lut-apply', label: lutWords.apply, disabled: !canApply || !lutUsable(lut), run: () => onApply(lut.name) },
      { id: 'lut-favorite', label: liked ? lutWords.removeFavorite : lutWords.addFavorite, disabled: lut.shadowed, run: () => toggleFavorite(lut.name) },
    ] });
  };
  const blankMenu = (event: ReactMouseEvent) => {
    event.preventDefault();
    setMenu({ x: event.clientX, y: event.clientY, label: lutWords.menu, items: [{ id: 'lut-refresh', label: lutWords.refresh, run: refresh }] });
  };
  const closeMenu = useCallback(() => setMenu(null), []);
  // The option menu (…): Live Preview, and Hover Scrub Preview ▸ (pp. 3259, 3411).
  const optionMenu = (event: ReactMouseEvent<HTMLButtonElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    setMenu({ x: box.left, y: box.bottom, label: lutWords.options, items: [
      { id: 'lut-live-preview', label: lutWords.livePreview, checked: options.live, run: () => choose({ live: !options.live }) },
      { id: 'lut-hover-scrub', label: lutWords.hoverScrub, items: HOVER_SCRUBS.map(scrub => ({
        id: `lut-scrub-${scrub}`, label: lutWords.scrubs[scrub], checked: options.scrub === scrub, run: () => choose({ scrub }),
      })) },
    ] });
  };
  const thumbnails = useLutThumbnails({ bridge, session, native, luts, interpolation });
  const live = useLivePreview({ bridge, session, native, place, options });
  const across = (event: ReactMouseEvent<HTMLElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    return box.width > 0 ? Math.min(1, Math.max(0, (event.clientX - box.left) / box.width)) : 0;
  };

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
      <button type="button" className="icon-btn" aria-label={lutWords.options} title={lutWords.options} aria-haspopup="menu"
        data-live={options.live || undefined} onClick={optionMenu} data-test="lut-options"><Ellipsis size={13} /></button>
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
                const broken = lutBroken(lut);
                const why = lut.shadowed ? said.lutShadowed : broken ? [said.lutBroken, ...(lut.message ? [lut.message] : [])].join('\n') : null;
                const usable = lutUsable(lut);
                // The current clip with the LUT under the pointer, in Live Preview; else the test image.
                const picture = live.picture?.lut === lut.name ? live.picture.url : thumbnails.get(thumbnailKey(session, interpolation, lut)) ?? null;
                return <li key={keyOf(lut)} className="color-lut" data-key={keyOf(lut)} data-lut={lut.name} aria-selected={chosen === keyOf(lut)}
                  data-shadowed={lut.shadowed || undefined} data-broken={broken || undefined} data-kind={lutDimension(lut)}
                  title={why === null ? lut.name : `${lut.name}\n${why}`}
                  draggable={usable} onDragStart={event => drag(lut, event)} onClick={() => setChosen(keyOf(lut))}
                  onContextMenu={event => lutMenu(lut, event)} data-test="lut">
                  <span className="color-lut-picture" aria-hidden="true" data-live={(live.on && usable) || undefined}
                    onPointerEnter={usable && live.on ? event => live.enter(lut.name, across(event)) : undefined}
                    onPointerMove={usable && live.on ? event => live.move(lut.name, across(event)) : undefined}
                    onPointerLeave={live.on ? live.leave : undefined} data-test="lut-picture">
                    {broken ? <TriangleAlert size={22} /> : picture !== null
                      ? <img src={picture} alt="" draggable={false} data-test="lut-thumbnail" /> : <Layers3 size={22} />}
                  </span>
                  <span className="color-lut-kind" lang="en" data-test="lut-kind">{lutDimension(lut) === '1d' ? '1D' : '3D'}</span>
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
