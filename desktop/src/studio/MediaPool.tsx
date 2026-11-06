import { useFairlightText } from './fairlight/words';
import { memo, useCallback, useEffect, useMemo, useRef, useState, type MouseEvent } from 'react';
import { Grid2X2, Import, List, RotateCw, Trash2, X } from 'lucide-react';
import { useWords } from '../app/locale';
import { studioSupported } from '../app/system';
import type { EngineBridge } from '../bridge';
import { callEngine, EngineError, keptColor, type Bin, type ColorSettings, type Media, type Op, type ProxyEvent, type Rate } from './engine';
import type { Menu } from './color/ColorMenu';
import { CoveringMenu } from './color/CoveringMenu';
import { MEDIA_COLOR_REASONS, mediaColorItems } from './media/colorMenu';
import { useFavoriteLuts, useListedLuts } from './color/useLuts';
import { Bins } from './media/Bins';
import { Clips } from './media/Clips';
import { Details } from './media/Details';
import { AudioAttributes, AudioFormats } from './media/AudioAttributes';
import { audioSources, standardLayout } from './media/audioLayout';
import type { TrackFormat } from './engine';
import { EMPTY_SELECTION, type Selection } from './media/selection';
import { Thumbnails } from './media/Thumbnails';
export { factsOf, proxyStanding } from './media/facts';

/** Media IDs and metadata are edited through one engine transaction; the original files stay where they are. */
export const MediaPool = memo(function MediaPool({ media, bins, timelines, proxies, loaded, bridge, session, onImport, onLoad, onMakeAgain, initialGrid = false, onBin, follow = null, color, audioFilters = false }: {
  media: readonly Media[] | null; bins: readonly Bin[]; timelines: readonly { rate: Rate }[];
  proxies: Readonly<Record<string, ProxyEvent>>; loaded: string | null; bridge: Pick<EngineBridge, 'call'>; session: number;
  onImport: (bin: string | null) => void; onLoad: (id: string) => void; onMakeAgain: (id: string) => void;
  /** Whether it opens in the thumbnail view, as the Cut page's pool does. */
  initialGrid?: boolean;
  /** Told of the bin shown (null for the master bin), whose media the source tape of the Cut page plays. */
  onBin?: (bin: string | null) => void;
  /** A medium to choose as it changes: the one under the playhead of the Cut page's source tape. */
  follow?: string | null;
  /** The colour management of the project, which says what the menu of media offers (its Input Color Space, p. 230). */
  color?: ColorSettings;
  /** Fairlight's three views of the shared pool; only its visible list is filtered. */
  audioFilters?: boolean;
}) {
  const words = useWords();
  const t = useFairlightText();
  const labels = words.label.studio.media;
  const [bin, setBin] = useState<string | null>(null);
  const [selection, setSelection] = useState<Selection>(EMPTY_SELECTION);
  const [grid, setGrid] = useState(initialGrid);
  const [audioFilter, setAudioFilter] = useState('all');
  const [problem, setProblem] = useState<{ said: string; detail: string } | null>(null);
  // The right-click menu: where, and of which media; its items are made as it is drawn, so the LUTs listed meanwhile show.
  const [menuOf, setMenu] = useState<{ x: number; y: number; of: Media[] } | null>(null);
  const pending = useRef<{ session: number } | null>(null);
  const currentSession = useRef(session);
  currentSession.current = session;
  const [audioOpened, setAudioOpened] = useState<{ session: number; ids: string[] } | null>(null);
  useEffect(() => { setAudioOpened(null); setMenu(null); setProblem(null); }, [session]);
  const selectedBin = bins.some(each => each.id === bin) ? bin : null;
  useEffect(() => { onBin?.(selectedBin); }, [selectedBin, onBin]);
  useEffect(() => { if (follow !== null) setSelection({ ids: [follow], focus: follow, anchor: follow }); }, [follow]);
  const visible = useMemo(() => (media ?? []).filter(each => each.bin === selectedBin && (!audioFilters || audioFilter === 'all' ||
    (each.facts.audio !== null && (audioFilter === 'with-audio' || each.facts.video === null)))), [media, selectedBin, audioFilters, audioFilter]);
  const chosen = visible.filter(each => selection.ids.includes(each.id));
  const details = chosen.find(each => each.id === selection.focus) ?? chosen[0];
  const failed = chosen.find(each => proxies[each.id]?.state === 'failed');
  const thumbnails = useMemo(() => new Thumbnails(bridge), [bridge, session]);
  const edit = useCallback(async (ops: Op[]): Promise<boolean> => {
    if (ops.length === 0 || pending.current?.session === session || currentSession.current !== session) return false;
    const request = { session };
    pending.current = request;
    setProblem(null);
    try {
      await callEngine(bridge, 'transaction.apply', { ops }, { expectedSession: session });
      return currentSession.current === session;
    } catch (error) {
      const reason = error instanceof EngineError ? (error.data as { reason?: string } | null)?.reason : null;
      const said = reason === 'not_empty' ? words.hint.studio.binUsed
        : reason === 'in_use' ? words.hint.studio.mediaUsed
          : MEDIA_COLOR_REASONS.find(each => each === reason) ? words.hint.studio.edit[reason as typeof MEDIA_COLOR_REASONS[number]]
            : words.hint.studio.mediaEditFailed;
      if (currentSession.current === session) setProblem({ said, detail: error instanceof Error ? error.message : String(error) });
      return false;
    } finally { if (pending.current === request) pending.current = null; }
  }, [bridge, words, session]);
  const selectBin = (id: string | null) => { setBin(id); setSelection(EMPTY_SELECTION); };
  const move = (ids: string[], target: string | null) => {
    const moving = (media ?? []).filter(each => ids.includes(each.id) && each.bin !== target);
    // Media already in that bin does not move; the engine refuses a step that changes nothing.
    if (moving.length === 0) return;
    void edit(moving.map(each => ({ op: 'set_media', id: each.id, bin: target })));
  };
  // The LUTs of the source clips' 1D LUT ▸ and 3D LUT ▸ (pp. 3409–3410), listed as the menu is first opened.
  const listedLuts = useListedLuts(bridge);
  const favoriteLuts = useFavoriteLuts();
  // The right-click menu of media: its colour, for those chosen where it is one of them, else for it alone, which is then chosen.
  const colorMenu = (event: MouseEvent) => {
    const id = (event.target as Element).closest<HTMLElement>('[data-media]')?.dataset.media;
    if (!id || !studioSupported()) return;
    event.preventDefault();
    const of = selection.ids.includes(id) ? visible.filter(each => selection.ids.includes(each.id)) : visible.filter(each => each.id === id);
    if (!selection.ids.includes(id)) setSelection({ ids: [id], focus: id, anchor: id });
    listedLuts.ensure();
    setMenu({ x: event.clientX, y: event.clientY, of });
  };
  const closeMenu = useCallback(() => setMenu(null), []);
  const audioTargets = (items: readonly Media[]) => {
    const audio = items.filter(item => item.facts.audio !== null && item.facts.audio !== undefined);
    return [...audio.filter(item => item.id !== details?.id), ...audio.filter(item => item.id === details?.id)];
  };
  const openAudio = (items: readonly Media[]) => setAudioOpened({ session, ids: audioTargets(items).map(item => item.id) });
  const openedMedia = audioOpened?.ids.map(id => media?.find(item => item.id === id)).filter((item): item is Media => !!item) ?? [];
  const selectedAudio = audioTargets(chosen);
  const fileFormat = details?.audio_config?.length === 1 ? details.audio_config[0].format
    : details?.audio_config ? 'custom' : details?.facts.audio?.channels === 1 ? 'mono' : details?.facts.audio?.channels === 2 ? 'stereo' : details?.facts.audio?.channels === 6 ? '5.1' : 'adaptive';
  const menu: Menu | null = menuOf === null ? null : {
    x: menuOf.x, y: menuOf.y, label: labels.menu, items: [...mediaColorItems(menuOf.of, keptColor({ color }), labels, ops => void edit(ops),
      { library: listedLuts.library, favorites: favoriteLuts, words: words.label.studio.colorPage.lutPanel }),
      { id: 'clip-attributes', label: t('Clip Attributes'), disabled: !audioTargets(menuOf.of).length, run: () => openAudio(menuOf.of) }],
  };
  return <section className="pool" aria-label={words.label.studio.pool} data-test="pool">
    <header className="pool-head">
      <h2 className="overline">{words.label.studio.pool}</h2>
      <div className="pool-toolbar">
        <button type="button" className="icon-btn" aria-label={labels.list} title={labels.list} aria-pressed={!grid} onClick={() => setGrid(false)}><List size={14} /></button>
        <button type="button" className="icon-btn" aria-label={labels.thumbnails} title={labels.thumbnails} aria-pressed={grid} onClick={() => setGrid(true)}><Grid2X2 size={14} /></button>
        <button type="button" className="btn btn-quiet" onClick={() => onImport(selectedBin)} data-test="import"><Import size={14} aria-hidden="true" />{words.label.studio.import}</button>
      </div>
    </header>
    {audioFilters && <select aria-label={t('Media audio filter')} value={audioFilter} onChange={event => setAudioFilter(event.target.value)}>
      <option value="all">{t('All clips')}</option><option value="audio-only">{t('Audio only')}</option><option value="with-audio">{t('Clips with audio')}</option>
    </select>}
    {problem && <div className="pool-problem" role="alert"><p>{problem.said}<span className="caption">{problem.detail}</span></p>
      <button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => setProblem(null)}><X size={14} /></button></div>}
    <div className="pool-browser">
      <Bins bins={bins} selected={selectedBin} onSelect={selectBin} onEdit={edit} onMove={move} session={session} />
      <div className="pool-clips" onContextMenu={colorMenu}>
        {media !== null && visible.length === 0 && <p className="caption pool-none" data-test="pool-none">{media.length === 0 ? words.hint.studio.noMedia : words.hint.studio.emptyBin}</p>}
        <Clips media={visible} timelines={timelines} proxies={proxies} loaded={loaded} selection={selection} onSelect={setSelection}
          onLoad={onLoad} onMakeAgain={onMakeAgain} onEdit={edit} session={session} thumbnails={grid ? thumbnails : null} />
      </div>
    </div>
    {chosen.length > 0 && <footer className="pool-foot">
      <span className="caption">{labels.selected(chosen.length)}</span>
      <button type="button" className="icon-btn" aria-label={labels.remove} title={labels.remove}
        onClick={() => void edit(chosen.map(each => ({ op: 'remove_media', id: each.id })))}><Trash2 size={14} /></button>
      {failed && <button type="button" className="btn btn-quiet" onClick={() => onMakeAgain(failed.id)} data-test="make-proxy-again"><RotateCw size={14} aria-hidden="true" />{words.label.studio.makeProxyAgain}</button>}
    </footer>}
    {details && <Details media={details} told={proxies[details.id]} timelines={timelines} onMakeAgain={() => onMakeAgain(details.id)} />}
    {details?.facts.audio && <section className="audio-config" aria-label={t('Audio Configuration')}>
      <label>{t('Format')}<select aria-label={t('Format')} value={fileFormat} onChange={event => {
        if (event.target.value === 'custom') openAudio(chosen);
        else if (selectedAudio.every(item => audioSources(item).length > 0)) void edit(selectedAudio.map(item => ({
          op: 'set_media_audio', id: item.id,
          config: standardLayout(item, event.target.value as TrackFormat),
        })));
      }}><AudioFormats disabled={!selectedAudio.every(item => audioSources(item).length > 0)} /><option value="custom">{t('Custom mapping')}</option></select></label>
      {!selectedAudio.every(item => audioSources(item).length > 0) && <p role="status">{t('Audio stream information is unavailable.')}</p>}
      <button type="button" onClick={() => openAudio(chosen)}>{t('Clip Attributes')}</button>
    </section>}
    {audioOpened && audioOpened.session === session && openedMedia.length > 0 && openedMedia.length === audioOpened.ids.length &&
      <AudioAttributes media={openedMedia} onApply={edit} onClose={() => setAudioOpened(null)} />}
    {menu && <CoveringMenu menu={menu} onClose={closeMenu} />}
  </section>;
});
