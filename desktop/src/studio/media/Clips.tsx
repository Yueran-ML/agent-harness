import { memo, useCallback, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react';
import { AudioLines, Film, Image } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Media, Op, ProxyEvent, Rate } from '../engine';
import { factsOf, ProxyLine, proxyStanding } from './facts';
import { MEDIA_DRAG } from './drag';
import { NameField } from './NameField';
import { choose, type Selection } from './selection';
import { Thumbnail, type Thumbnails } from './Thumbnails';

const Row = memo(function Row({ media, told, timelines, chosen, loaded, tabbable, thumbnails, onChoose, onLoad, onMakeAgain, onDrag, rename, onRename, onCancel }: {
  media: Media; told: ProxyEvent | undefined; timelines: readonly { rate: Rate }[];
  chosen: boolean; loaded: boolean; tabbable: boolean; thumbnails: Thumbnails | null;
  onChoose: (id: string, event: MouseEvent) => void; onLoad: (id: string) => void; onMakeAgain: (id: string) => void;
  onDrag: (id: string, data: DataTransfer) => void; rename: boolean; onRename: (id: string, name: string) => Promise<boolean>; onCancel: () => void;
}) {
  const words = useWords();
  const facts = useMemo(() => factsOf(words, media, timelines).join(' · '), [words, media, timelines]);
  const Icon = media.facts.video?.still ? Image : media.facts.video ? Film : AudioLines;
  return <li role="option" className="pool-item" aria-selected={chosen} aria-current={loaded || undefined} tabIndex={tabbable ? 0 : -1}
    title={media.path} onClick={event => onChoose(media.id, event)} onDoubleClick={() => onLoad(media.id)}
    draggable={!rename} onDragStart={event => onDrag(media.id, event.dataTransfer)} data-test="pool-item" data-media={media.id}>
    {thumbnails && media.facts.video ? <Thumbnail media={media} cache={thumbnails} /> : <Icon size={thumbnails ? 24 : 15} aria-hidden="true" />}
    <span className="pool-words">
      {rename ? <NameField name={media.name} onCommit={name => onRename(media.id, name)} onCancel={onCancel} />
        : <span className="pool-name" data-test="pool-name">{media.name}</span>}
      <span className="pool-facts" data-test="pool-facts">{facts}</span>
      <ProxyLine standing={proxyStanding(media, told)} onMakeAgain={() => onMakeAgain(media.id)} />
    </span>
  </li>;
});

export function Clips({ media, timelines, proxies, loaded, selection, onSelect, onLoad, onMakeAgain, onEdit, session, thumbnails }: {
  media: readonly Media[]; timelines: readonly { rate: Rate }[]; proxies: Readonly<Record<string, ProxyEvent>>; loaded: string | null;
  selection: Selection; onSelect: (selection: Selection) => void; onLoad: (id: string) => void; onMakeAgain: (id: string) => void;
  onEdit: (ops: Op[]) => Promise<boolean>; session: number; thumbnails: Thumbnails | null;
}) {
  const words = useWords();
  const list = useRef<HTMLUListElement>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const order = useMemo(() => media.map(each => each.id), [media]);
  const chosen = selection.ids.filter(id => order.includes(id));
  const focus = order.includes(selection.focus ?? '') ? selection.focus : order.includes(loaded ?? '') ? loaded : order[0];
  const pick = useCallback((id: string, event: MouseEvent) => {
    onSelect(choose(selection, order, id, event.ctrlKey || event.metaKey, event.shiftKey));
  }, [selection, order, onSelect]);
  const drag = useCallback((id: string, data: DataTransfer) => {
    const ids = selection.ids.includes(id) ? order.filter(each => selection.ids.includes(each)) : [id];
    if (!selection.ids.includes(id)) onSelect(choose(selection, order, id, false, false));
    data.setData(MEDIA_DRAG, JSON.stringify({ session, media: ids }));
    data.effectAllowed = 'copyMove';
  }, [selection, order, onSelect, session]);
  const rename = useCallback((id: string, name: string) => onEdit([{ op: 'set_media', id, name }]), [onEdit]);
  const cancel = useCallback(() => setRenaming(null), []);
  const keys = (event: KeyboardEvent) => {
    if ((event.target as HTMLElement).closest('input, textarea') || event.nativeEvent.isComposing) return;
    if (event.repeat && ['F2', 'Delete', 'Backspace', 'Enter'].includes(event.key)) { event.preventDefault(); return; }
    const current = (event.target as HTMLElement).closest<HTMLElement>('[data-media]')?.dataset.media ?? focus;
    if (!current) return;
    if (event.key === 'Enter') { event.preventDefault(); onLoad(current); }
    else if (event.key === 'F2') { event.preventDefault(); setRenaming(current); }
    else if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault();
      if (chosen.length) void onEdit(chosen.map(id => ({ op: 'remove_media', id })));
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
      event.preventDefault(); onSelect({ ids: [...order], focus: current, anchor: order[0] ?? null });
    } else if (['ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
      event.preventDefault();
      const rows = [...(list.current?.children ?? [])].filter((element): element is HTMLElement => element instanceof HTMLElement);
      const nextRow = thumbnails ? rows.findIndex(row => row.offsetTop !== rows[0]?.offsetTop) : -1;
      const columns = nextRow > 0 ? nextRow : thumbnails ? Math.max(1, rows.length) : 1;
      const distance = event.key === 'ArrowUp' || event.key === 'ArrowDown' ? columns : 1;
      const step = (event.key === 'ArrowUp' || event.key === 'ArrowLeft' ? -1 : 1) * distance;
      const at = event.key === 'Home' ? 0 : event.key === 'End' ? order.length - 1 : Math.max(0, Math.min(order.length - 1, order.indexOf(current) + step));
      const next = order[at];
      onSelect(choose(selection, order, next, event.ctrlKey || event.metaKey, event.shiftKey));
      rows.find(element => element.dataset.media === next)?.focus();
    }
  };
  return <ul ref={list} className="pool-list" data-layout={thumbnails ? 'grid' : 'list'} role="listbox" aria-multiselectable
    aria-label={words.label.studio.pool} onKeyDown={keys} data-test="pool-list">
    {media.map(each => <Row key={each.id} media={each} told={proxies[each.id]} timelines={timelines} chosen={chosen.includes(each.id)}
      loaded={loaded === each.id} tabbable={focus === each.id} thumbnails={thumbnails} onChoose={pick} onLoad={onLoad}
      onMakeAgain={onMakeAgain} onDrag={drag} rename={renaming === each.id} onRename={rename} onCancel={cancel} />)}
  </ul>;
}
