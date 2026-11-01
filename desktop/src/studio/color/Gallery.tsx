/**
 * The Gallery of the Color page (Reference Manual 20, ch. 141–142; the interaction reference,
 * §9.1, §9.2, §9.4): at the top left, where the LUT browser and the media pool also go.
 *
 * - The Album button shows the albums, the project's and then the PowerGrade albums at the bottom
 *   (p. 3265); a click makes one current, a double-click renames it, and its right-click menu adds
 *   a still album or a PowerGrade album, or removes the current one once that is confirmed.
 * - The stills of the current album, as large as the Still Size slider makes them, each with its
 *   number and its label beneath, and those whose label holds what is searched for (p. 3263). A
 *   click chooses one, Ctrl+click and Shift+click more (p. 3258); a double-click wipes it (p. 3260);
 *   the middle button applies its grade to the clips chosen (p. 3288); a still dragged onto an
 *   album is moved there, and onto a memory is copied into it (pp. 3265, 3272).
 * - The right-click menu of a still: Apply Grade, Append Node Graph, Delete Selected (not confirmed:
 *   Ctrl+Z while the Gallery has the focus brings the stills deleted last back, the interaction
 *   reference §9.2), Properties, Change Label, Export; of the Gallery behind them: Apply Grades
 *   Using ▸, Copy Grade: Preserve number of nodes ▸, Import.
 * - The Memories button shows the memories A–H at the foot (pp. 3271–3272): Alt+1…8 grabs the
 *   current clip's grade into one, Ctrl+1…8 applies it, as does the middle button; its menu applies,
 *   appends or clears it.
 * - Grab Still (Ctrl+Alt+G) puts a still of the frame shown into the current album.
 */
import {
  useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactNode,
} from 'react';
import { Bookmark, Images, Search } from 'lucide-react';
import { Sheet } from '../../app/Sheet';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import {
  callEngine, EngineError, type Clip, type GalleryAlbum, type Group, type Still, type StillWhole, type Target, type Timeline,
} from '../engine';
import { refusalOf } from '../inspector/refusal';
import { ColorMenu, type Menu, type MenuItem } from './ColorMenu';
import type { GradeLevel } from './clips';
import {
  applyParams, applyTargets, grabParams, KEYFRAMES_USING, keepApplySettings, keepMemories, MEMORY_LETTERS, pickStills, PRESERVE_MOST,
  readApplySettings, readMemories, searched, STILL_SIZES, stepStill, thumbnailHeight, useGallery, type ApplySettings, type Memories,
} from './stills';
import './gallery.css';

type GalleryBridge = Pick<EngineBridge, 'call' | 'onEvent'> & Partial<Pick<EngineBridge, 'importStills' | 'exportStills'>>;

/** What is asked to be confirmed, or set, in a window over the page. */
type Asking =
  | { kind: 'removeAlbum'; album: string }
  | { kind: 'label'; still: string; value: string }
  | { kind: 'properties'; still: StillWhole };

export interface ColorGallery {
  /** The panel, to be laid where the gallery goes. */
  panel: ReactNode;
  /** Grab Still (Ctrl+Alt+G, the viewer's menu). */
  grab: () => void;
  /** Previous / Next Still (Ctrl+Alt+B / N): the still chosen, which is returned. */
  stepStill: (direction: -1 | 1) => string | null;
  /** Save Memory A–H (Alt+1…8) and Load Memory A–H (Ctrl+1…8). */
  storeMemory: (slot: number) => void;
  recallMemory: (slot: number) => void;
  /** Apply Grade or Append Node Graph of a still to the clips chosen (the viewer's Apply Grade ▸ too). */
  apply: (still: string, mode: 'apply' | 'append', keyframes?: ApplySettings['keyframes']) => void;
  /** The still chosen last, and those chosen, in the order of the album. */
  currentStill: string | null;
  chosenStills: string[];
  settings: ApplySettings;
}

const wholeOf = (stills: readonly Still[], id: string | null) => (id === null ? undefined : stills.find(still => still.id === id));

/** A still's thumbnail, asked for at the height that the slider makes it. */
function StillPicture({ still, height, thumbnail }: { still: Still; height: number; thumbnail: (still: string, height: number) => Promise<string | null> }) {
  const [picture, setPicture] = useState<string | null>(null);
  const asked = thumbnailHeight(height, typeof devicePixelRatio === 'number' ? devicePixelRatio : 1);
  useEffect(() => {
    let gone = false;
    void thumbnail(still.id, asked).then((found) => { if (!gone) setPicture(found); });
    return () => { gone = true; };
  }, [still.id, asked, thumbnail]);
  const width = still.image.height > 0 ? Math.round((height * still.image.width) / still.image.height) : Math.round((height * 16) / 9);
  return <span className="gallery-picture" style={{ width, height }}>
    {picture ? <img src={picture} alt="" draggable={false} /> : <Images size={16} aria-hidden="true" />}
  </span>;
}

export function useColorGallery({ bridge, session, project, language, revision, level, timeline, clips, chosenClips, current, groups, wipedStill,
  onWipe, onRefused, onFailed }: {
  bridge: GalleryBridge;
  session: number;
  /** The folder of the project, which the memories are kept by. */
  project: string;
  language: string;
  revision: number;
  /** The Node Editor's mode, which decides what a grab takes and what an apply goes to. */
  level: GradeLevel;
  timeline: Timeline | null;
  /** The clips of the thumbnail timeline in its order, those chosen in it, and the current one. */
  clips: readonly Clip[];
  chosenClips: ReadonlySet<string>;
  current: Clip | null;
  groups: readonly Group[];
  /** The still that the wipe shows now, marked in the Gallery. */
  wipedStill: string | null;
  /** A still double-clicked: it is wiped (p. 3260). */
  onWipe: (still: string) => void;
  /** Why the engine refused to change grades, said where the node editor says it. */
  onRefused: (text: string) => void;
  onFailed: (failure: unknown) => void;
}): ColorGallery {
  const words = useWords();
  const labels = words.label.studio.colorPage.galleryPanel;
  const said = words.hint.studio.colorPage.gallery;
  const gallery = useGallery(bridge, session);
  const [albumsShown, setAlbumsShown] = useState(true);
  // Memories are hidden at first (p. 3271).
  const [memoriesShown, setMemoriesShown] = useState(false);
  const [size, setSize] = useState<number>(STILL_SIZES.initial);
  const [search, setSearch] = useState('');
  const [searching, setSearching] = useState(false);
  const [albumId, setAlbumId] = useState<string | null>(null);
  const [chosen, setChosen] = useState<string[]>([]);
  const [currentStill, setCurrentStill] = useState<string | null>(null);
  const anchor = useRef<string | null>(null);
  const [renaming, setRenaming] = useState<{ album: string; value: string } | null>(null);
  const [menu, setMenu] = useState<Menu | null>(null);
  const [asking, setAsking] = useState<Asking | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  // What was said of the last deletion: that Ctrl+Z brings it back.
  const [notice, setNotice] = useState<string | null>(null);
  // The stills of each deletion of this session, the last last, that Ctrl+Z in the Gallery restores.
  const deletions = useRef<string[][]>([]);
  const [settings, setSettingsState] = useState<ApplySettings>(readApplySettings);
  const [memories, setMemoriesState] = useState<Memories>(() => readMemories(project));
  const dragged = useRef<string[] | null>(null);
  useEffect(() => { setMemoriesState(readMemories(project)); }, [project]);
  useEffect(() => { setChosen([]); setCurrentStill(null); setProblem(null); setNotice(null); deletions.current = []; }, [session]);
  const setSettings = (next: ApplySettings) => { setSettingsState(next); keepApplySettings(next); };
  const setMemories = useCallback((next: Memories) => { setMemoriesState(next); keepMemories(project, next); }, [project]);

  // The albums that the list shows: the engine lists the album of the memories apart.
  const listed = gallery.albums;
  const album: GalleryAlbum | null = listed.find(each => each.id === albumId) ?? listed[0] ?? null;
  const shown = useMemo(() => searched(album?.stills ?? [], search), [album, search]);
  const everyStill = useMemo(() => [...gallery.albums, ...(gallery.memories ? [gallery.memories] : [])].flatMap(each => each.stills),
    [gallery.albums, gallery.memories]);
  // Stills that are gone are no longer chosen.
  useEffect(() => {
    if (!gallery.loaded) return;
    const there = new Set(everyStill.map(still => still.id));
    setChosen(old => (old.every(id => there.has(id)) ? old : old.filter(id => there.has(id))));
    setCurrentStill(old => (old !== null && !there.has(old) ? null : old));
  }, [everyStill, gallery.loaded]);

  const now = useRef({ bridge, revision, level, timeline, clips, chosenClips, current, groups, settings, memories, album, shown, chosen, currentStill, language, labels });
  now.current = { bridge, revision, level, timeline, clips, chosenClips, current, groups, settings, memories, album, shown, chosen, currentStill, language, labels };

  /** Say why a change of the Gallery was refused, in the panel. */
  const refused = useCallback((failure: unknown) => {
    if (!(failure instanceof EngineError)) { onFailed(failure); return; }
    const overrides: Record<string, string> = { ...said.refusals };
    setProblem(refusalOf(failure, words.hint.studio, overrides).text);
  }, [said, words, onFailed]);
  const galleryCall = useCallback(async <T,>(asking: Promise<T>): Promise<T | null> => {
    setProblem(null);
    setNotice(null);
    try { return await asking; } catch (failure) { refused(failure); return null; }
  }, [refused]);

  const grab = useCallback(() => {
    const state = now.current;
    if (state.timeline === null) return;
    void galleryCall(callEngine(state.bridge, 'still.grab', grabParams(state.album?.id ?? null, state.current, state.level)));
  }, [galleryCall]);

  const apply = useCallback((still: string, mode: 'apply' | 'append', keyframes?: ApplySettings['keyframes']) => {
    const state = now.current;
    const to: Target[] = applyTargets(state.level, state.timeline, state.clips, state.chosenClips, state.current, state.groups);
    if (to.length === 0) return;
    const using = keyframes === undefined ? state.settings : { ...state.settings, keyframes };
    callEngine(state.bridge, 'still.apply', applyParams(still, to, mode, using, state.revision)).catch((failure: unknown) => {
      if (failure instanceof EngineError) onRefused(refusalOf(failure, words.hint.studio, { ...said.refusals }).text);
      else onFailed(failure);
    });
  }, [onRefused, onFailed, said, words]);

  const step = useCallback((direction: -1 | 1) => {
    const state = now.current;
    const next = stepStill(state.shown, state.currentStill, direction);
    if (next === null) return null;
    setCurrentStill(next.id); setChosen([next.id]); anchor.current = next.id;
    return next.id;
  }, []);

  /** A still kept in a memory no longer, where only the memory had it: taken out of the album of the memories. */
  const forget = useCallback((still: string | null, slots: (string | null)[]) => {
    const state = now.current;
    if (still === null || slots.includes(still)) return;
    const kept = gallery.memories?.stills.some(each => each.id === still);
    if (kept) void callEngine(state.bridge, 'still.delete', { stills: [still] }).catch(() => {});
  }, [gallery.memories]);

  /** The project's album of the memories, made the first time a memory is kept (`gallery.album.create` of kind `memories`). */
  const memoryAlbum = useCallback(async (): Promise<string | null> => {
    if (gallery.memories) return gallery.memories.id;
    const made = await galleryCall(callEngine(now.current.bridge, 'gallery.album.create', { kind: 'memories', name: now.current.labels.memoriesAlbum }));
    return made === null ? null : made.id;
  }, [galleryCall, gallery.memories]);

  /** Keep `still` in memory `slot`, the still it held forgotten. */
  const keepInMemory = useCallback((slot: number, still: string, album: string) => {
    const old = now.current.memories.slots[slot];
    const slots = now.current.memories.slots.map((each, at) => (at === slot ? still : each));
    setMemories({ album, slots });
    forget(old, slots);
  }, [setMemories, forget]);

  const storeMemory = useCallback((slot: number) => {
    const state = now.current;
    if (state.timeline === null || slot < 0 || slot >= MEMORY_LETTERS.length) return;
    void (async () => {
      const album = await memoryAlbum();
      if (album === null) return;
      const still = await galleryCall(callEngine(state.bridge, 'still.grab', grabParams(album, state.current, state.level)));
      if (still !== null) keepInMemory(slot, still.id, album);
    })();
  }, [galleryCall, memoryAlbum, keepInMemory]);

  const recallMemory = useCallback((slot: number) => {
    const still = now.current.memories.slots[slot];
    if (still) apply(still, 'apply');
  }, [apply]);

  const clearMemory = (slot: number) => {
    const old = memories.slots[slot];
    const slots = memories.slots.map((each, at) => (at === slot ? null : each));
    setMemories({ ...memories, slots });
    forget(old, slots);
  };

  const pick = (still: string, event: ReactMouseEvent) => {
    const next = pickStills(shown, chosen, anchor.current, still, { ctrl: event.ctrlKey || event.metaKey, shift: event.shiftKey });
    setChosen(next);
    if (!event.shiftKey) anchor.current = still;
    setCurrentStill(still);
  };

  const exportStills = async (stills: string[]) => {
    if (!bridge.exportStills) return;
    setProblem(null);
    try {
      const answer = await bridge.exportStills({ language, stills });
      if (!answer.ok) refused(new EngineError(answer.error));
    } catch (failure) { onFailed(failure); }
  };
  const importStills = async () => {
    if (!bridge.importStills) return;
    setProblem(null);
    try {
      const answer = await bridge.importStills({ language, ...(album === null ? {} : { album: album.id }) });
      if (!answer.ok) refused(new EngineError(answer.error));
    } catch (failure) { onFailed(failure); }
  };
  const properties = async (still: string) => {
    const whole = await galleryCall(callEngine(bridge, 'still.get', { still }));
    if (whole !== null) setAsking({ kind: 'properties', still: whole });
  };

  /** Delete Selected (p. 3258), not confirmed: Ctrl+Z in the Gallery brings them back (§9.2). */
  const deleteStills = async (stills: string[]) => {
    const done = await galleryCall(callEngine(bridge, 'still.delete', { stills }));
    if (done === null) return;
    deletions.current = [...deletions.current, done.deleted];
    setChosen([]);
    setNotice(said.deleted);
  };
  /** Ctrl+Z while the Gallery has the focus: the stills deleted last back where they were. Whether there were any. */
  const restoreLast = () => {
    const stills = deletions.current.at(-1);
    if (stills === undefined) return false;
    deletions.current = deletions.current.slice(0, -1);
    void galleryCall(callEngine(bridge, 'still.restore', { stills })).then((done) => {
      if (done !== null) { setChosen(stills); setCurrentStill(stills.at(-1) ?? null); }
    });
    return true;
  };
  const galleryKey = (event: ReactKeyboardEvent) => {
    const target = event.target as HTMLElement;
    if (target.closest('input, textarea, select, [contenteditable="true"]')) return;
    const z = event.code === 'KeyZ' || event.key.toLowerCase() === 'z';
    if (!z || !(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey) return;
    if (restoreLast()) { event.preventDefault(); event.stopPropagation(); }
  };

  const stillItems = (still: string): MenuItem[] => {
    const of = chosen.includes(still) ? chosen : [still];
    const canApply = timeline !== null;
    return [
      { id: 'apply-grade', label: labels.applyGrade, disabled: !canApply, run: () => apply(still, 'apply') },
      { id: 'append-graph', label: labels.appendNodeGraph, disabled: !canApply, run: () => apply(still, 'append') },
      { id: 'delete-selected', label: labels.deleteSelected, run: () => void deleteStills(of) },
      { id: 'properties', label: labels.properties, run: () => void properties(still) },
      { id: 'change-label', label: labels.changeLabel, run: () => setAsking({ kind: 'label', still, value: wholeOf(everyStill, still)?.label ?? '' }) },
      { id: 'export', label: labels.export, disabled: !bridge.exportStills, run: () => void exportStills(of) },
    ];
  };
  const galleryItems = (): MenuItem[] => [
    { id: 'apply-grades-using', label: labels.applyGradesUsing, items: KEYFRAMES_USING.map(each => ({
      id: `keyframes-${each}`, label: labels.keyframes[each], checked: settings.keyframes === each, run: () => setSettings({ ...settings, keyframes: each }),
    })) },
    { id: 'preserve-nodes', label: labels.preserveNodes, items: Array.from({ length: PRESERVE_MOST + 1 }, (_, count) => ({
      id: `preserve-${count}`, label: String(count), checked: settings.preserve === count, run: () => setSettings({ ...settings, preserve: count }),
    })) },
    { id: 'import', label: labels.import, disabled: !bridge.importStills, run: () => void importStills() },
  ];
  const albumItems = (): MenuItem[] => [
    { id: 'add-still-album', label: labels.addStillAlbum, run: () => void galleryCall(callEngine(bridge, 'gallery.album.create', { kind: 'stills' })) },
    { id: 'add-powergrade-album', label: labels.addPowerGradeAlbum,
      run: () => void galleryCall(callEngine(bridge, 'gallery.album.create', { kind: 'powergrade' })) },
    { id: 'remove-album', label: labels.removeAlbum, disabled: album === null, run: () => { if (album) setAsking({ kind: 'removeAlbum', album: album.id }); } },
  ];
  const memoryItems = (slot: number): MenuItem[] => {
    const still = memories.slots[slot];
    return [
      { id: 'apply-grade', label: labels.applyGrade, disabled: still === null || timeline === null, run: () => { if (still) apply(still, 'apply'); } },
      { id: 'append-graph', label: labels.appendNodeGraph, disabled: still === null || timeline === null, run: () => { if (still) apply(still, 'append'); } },
      { id: 'clear-memory', label: labels.clearMemory, disabled: still === null, run: () => clearMemory(slot) },
    ];
  };
  const openMenu = (event: ReactMouseEvent, label: string, items: MenuItem[]) => {
    event.preventDefault();
    event.stopPropagation();
    setMenu({ x: event.clientX, y: event.clientY, label, items });
  };

  const startDrag = (event: DragEvent, still: string) => {
    const of = chosen.includes(still) ? chosen : [still];
    dragged.current = of;
    try {
      event.dataTransfer?.setData('text/plain', of.join(' '));
      if (event.dataTransfer) event.dataTransfer.effectAllowed = 'copyMove';
    } catch { /* A drag of the tests carries no data. */ }
  };
  const dropOnAlbum = (event: DragEvent, target: GalleryAlbum) => {
    event.preventDefault();
    const stills = dragged.current;
    dragged.current = null;
    if (!stills || stills.length === 0 || stills.every(id => target.stills.some(still => still.id === id))) return;
    void galleryCall(callEngine(bridge, 'still.move', { stills, album: target.id }));
  };
  /** A still dropped on a memory is copied into the album of the memories (p. 3272), so that the memory keeps it whatever is done to the still. */
  const dropOnMemory = (event: DragEvent, slot: number) => {
    event.preventDefault();
    const stills = dragged.current;
    dragged.current = null;
    if (!stills || stills.length === 0) return;
    void (async () => {
      const album = await memoryAlbum();
      if (album === null) return;
      const copied = await galleryCall(callEngine(bridge, 'still.copy', { stills: [stills[0]], album }));
      const copy = copied?.copied[stills[0]];
      if (copy) keepInMemory(slot, copy, album);
    })();
  };

  const confirm = async () => {
    const asked = asking;
    setAsking(null);
    if (asked?.kind === 'removeAlbum') {
      const done = await galleryCall(callEngine(bridge, 'gallery.album.delete', { album: asked.album }));
      if (done !== null) setAlbumId(null);
    } else if (asked?.kind === 'label') {
      await galleryCall(callEngine(bridge, 'still.label', { still: asked.still, label: asked.value.trim() }));
    }
  };
  const rename = () => {
    const asked = renaming;
    setRenaming(null);
    const name = asked?.value.trim() ?? '';
    const old = gallery.albums.find(each => each.id === asked?.album);
    if (!asked || !name || old?.name === name) return;
    void galleryCall(callEngine(bridge, 'gallery.album.rename', { album: asked.album, name }));
  };

  const closeMenu = useCallback(() => setMenu(null), []);
  const memoryStill = (slot: number) => wholeOf(everyStill, memories.slots[slot]);
  const asked = asking;
  const sheet = asked === null ? null
    : asked.kind === 'properties' ? <Sheet title={labels.propertiesTitle} onClose={() => setAsking(null)}>
      <dl className="gallery-properties" data-test="still-properties">
        {([
          [labels.stillId, asked.still.name || asked.still.id],
          [labels.labelField, asked.still.label],
          [labels.created, new Date(asked.still.created).toLocaleString(language)],
          [labels.timeline, asked.still.source?.timeline.name ?? ''],
          [labels.clip, asked.still.source?.media?.name ?? asked.still.source?.clip?.id ?? ''],
          [labels.recordTimecode, asked.still.source?.record_timecode ?? ''],
          [labels.sourceTimecode, asked.still.source?.source_timecode ?? ''],
          [labels.size, `${asked.still.image.width} × ${asked.still.image.height}`],
          ...(asked.still.imported ? [[labels.imported, asked.still.imported]] : []),
        ] as [string, string][]).map(([name, value]) => <div key={name}><dt>{name}</dt><dd>{value}</dd></div>)}
      </dl>
    </Sheet>
      : asked.kind === 'label' ? <Sheet title={labels.changeLabel} onClose={() => setAsking(null)} foot={<>
        <button type="button" className="btn" onClick={() => setAsking(null)}>{words.label.cancel}</button>
        <button type="button" className="btn btn-main" onClick={() => void confirm()} data-test="label-ok">{labels.ok}</button>
      </>}>
        <form onSubmit={(event) => { event.preventDefault(); void confirm(); }}>
          <label className="setting-name" htmlFor="still-label">{labels.labelField}</label>
          <input id="still-label" className="field" value={asked.value} maxLength={100} data-first autoComplete="off" spellCheck={false}
            onChange={event => setAsking({ ...asked, value: event.target.value })} data-test="still-label" />
        </form>
      </Sheet>
        : <Sheet title={labels.removeAlbum} onClose={() => setAsking(null)} foot={<>
          <button type="button" className="btn" onClick={() => setAsking(null)} data-first>{words.label.cancel}</button>
          <button type="button" className="btn btn-main" onClick={() => void confirm()} data-test="confirm-delete">{labels.delete}</button>
        </>}>
          <p data-test="confirm-words">{said.confirmRemoveAlbum}</p>
        </Sheet>;

  const panel = <div className="gallery" data-test="gallery" tabIndex={-1} onKeyDown={galleryKey}>
    <div className="gallery-head">
      <button type="button" className="icon-btn" aria-pressed={albumsShown} aria-label={labels.albums} title={labels.albums}
        onClick={() => setAlbumsShown(open => !open)} data-test="toggle-albums"><Images size={14} /></button>
      <button type="button" className="icon-btn" aria-pressed={memoriesShown} aria-label={labels.memories} title={labels.memories}
        onClick={() => setMemoriesShown(open => !open)} data-test="toggle-memories"><Bookmark size={14} /></button>
      <h2 className="overline gallery-title">{words.label.studio.colorPage.gallery}</h2>
      <input type="range" className="gallery-size" min={STILL_SIZES.least} max={STILL_SIZES.most} value={size} aria-label={labels.stillSize}
        title={labels.stillSize} onChange={event => setSize(Number(event.target.value))} data-test="still-size" />
      <button type="button" className="icon-btn" aria-pressed={searching} aria-label={labels.search} title={labels.search}
        onClick={() => { setSearching(open => !open); if (searching) setSearch(''); }} data-test="toggle-search"><Search size={14} /></button>
    </div>
    {searching && <input className="field gallery-search" type="search" value={search} aria-label={labels.search} placeholder={labels.search}
      onChange={event => setSearch(event.target.value)} data-test="still-search" autoFocus />}
    <div className="gallery-body">
      {albumsShown && <ul className="gallery-albums" role="listbox" aria-label={labels.albums} data-test="albums"
        onContextMenu={event => openMenu(event, labels.albumMenu, albumItems())}>
        {listed.map(each => <li key={each.id} role="option" aria-selected={each.id === album?.id} data-album={each.id} data-kind={each.kind}
          data-test="album" onClick={() => setAlbumId(each.id)}
          onContextMenu={() => setAlbumId(each.id)}
          onDoubleClick={() => setRenaming({ album: each.id, value: each.name })}
          onDragOver={(event) => { if (dragged.current) event.preventDefault(); }} onDrop={event => dropOnAlbum(event, each)}>
          {renaming?.album === each.id
            ? <input className="field gallery-rename" value={renaming.value} aria-label={labels.renameAlbum} autoFocus data-test="album-name"
              onChange={event => setRenaming({ album: each.id, value: event.target.value })}
              onKeyDown={(event) => {
                if (event.key === 'Enter') { event.preventDefault(); rename(); } else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setRenaming(null); }
              }}
              onBlur={() => setRenaming(null)} />
            : <span title={each.name}>{each.name}</span>}
        </li>)}
      </ul>}
      <div className="gallery-stills" role="listbox" aria-multiselectable aria-label={labels.stills} data-test="stills"
        onContextMenu={event => openMenu(event, labels.galleryMenu, galleryItems())}
        onClick={(event) => { if (event.target === event.currentTarget) { setChosen([]); } }}>
        {shown.map(still => <div key={still.id} role="option" className="gallery-still" aria-selected={chosen.includes(still.id)}
          data-still={still.id} data-current={still.id === currentStill || undefined} data-wiped={still.id === wipedStill || undefined} draggable
          data-test="still" title={still.label || still.name}
          onClick={event => pick(still.id, event)}
          onDoubleClick={() => { setCurrentStill(still.id); setChosen([still.id]); onWipe(still.id); }}
          onPointerDown={(event) => { if (event.button === 1) { event.preventDefault(); apply(still.id, 'apply'); } }}
          onContextMenu={(event) => {
            if (!chosen.includes(still.id)) { setChosen([still.id]); anchor.current = still.id; }
            setCurrentStill(still.id);
            openMenu(event, labels.stillMenu, stillItems(still.id));
          }}
          onDragStart={event => startDrag(event, still.id)} onDragEnd={() => { dragged.current = null; }}>
          <StillPicture still={still} height={size} thumbnail={gallery.thumbnail} />
          <span className="gallery-still-name" data-test="still-name">{still.name || still.id}</span>
          <span className="gallery-still-label" data-test="still-label-text"
            onDoubleClick={(event) => { event.stopPropagation(); setAsking({ kind: 'label', still: still.id, value: still.label }); }}>{still.label}</span>
        </div>)}
        {gallery.loaded && shown.length === 0 && <p className="caption color-empty" data-test="stills-empty">
          {gallery.error !== null ? said.failed : search.trim() !== '' && (album?.stills.length ?? 0) > 0 ? said.noneFound : said.empty}</p>}
      </div>
    </div>
    {memoriesShown && <ol className="gallery-memories" aria-label={labels.memories} data-test="memories">
      {MEMORY_LETTERS.map((letter, slot) => {
        const still = memoryStill(slot);
        return <li key={letter} className="gallery-memory" data-slot={letter} data-filled={still !== undefined || undefined} data-test="memory"
          title={`${labels.memory} ${letter}`}
          onPointerDown={(event) => { if (event.button === 1 && still) { event.preventDefault(); apply(still.id, 'apply'); } }}
          onContextMenu={event => openMenu(event, labels.memoryMenu, memoryItems(slot))}
          onDragOver={(event) => { if (dragged.current) event.preventDefault(); }} onDrop={event => dropOnMemory(event, slot)}>
          {still ? <StillPicture still={still} height={28} thumbnail={gallery.thumbnail} /> : <span className="gallery-memory-empty" />}
          <span className="gallery-memory-letter">{letter}</span>
        </li>;
      })}
    </ol>}
    {problem && <p className="gallery-problem" role="alert" data-test="gallery-problem">{problem}</p>}
    {!problem && notice && <p className="caption gallery-notice" role="status" data-test="gallery-notice">{notice}</p>}
    {menu && <ColorMenu menu={menu} onClose={closeMenu} />}
    {sheet}
  </div>;

  return {
    panel, grab, stepStill: step, storeMemory, recallMemory, apply, currentStill, settings,
    chosenStills: shown.filter(still => chosen.includes(still.id)).map(still => still.id),
  };
}
