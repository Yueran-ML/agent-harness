import { useRef, useState, type KeyboardEvent } from 'react';
import { ChevronDown, ChevronRight, Folder, FolderPlus, Trash2, Pencil } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Bin, Op } from '../engine';
import { draggedMedia, MEDIA_DRAG } from './drag';
import { NameField } from './NameField';

export function Bins({ bins, selected, onSelect, onEdit, onMove, session }: {
  bins: readonly Bin[]; selected: string | null; onSelect: (id: string | null) => void;
  onEdit: (ops: Op[]) => Promise<boolean>; onMove: (media: string[], bin: string | null) => void; session: number;
}) {
  const words = useWords().label.studio.media;
  const tree = useRef<HTMLDivElement>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState<{ id: string | null; parent: string | null; name: string } | null>(null);
  const toggle = (id: string) => setCollapsed(before => { const next = new Set(before); if (next.has(id)) next.delete(id); else next.add(id); return next; });
  const edit = () => { const current = bins.find(bin => bin.id === selected); if (current) setEditing({ id: current.id, parent: current.parent, name: current.name }); };
  const remove = () => { if (selected) void onEdit([{ op: 'remove_bin', id: selected }]); };
  const keys = (event: KeyboardEvent) => {
    if ((event.target as HTMLElement).closest('input')) return;
    if (event.key === 'F2') { event.preventDefault(); edit(); }
    else if (event.key === 'Delete' || event.key === 'Backspace') { event.preventDefault(); remove(); }
    else if (event.key.startsWith('Arrow')) {
      event.preventDefault();
      const rows = [...(tree.current?.querySelectorAll<HTMLElement>('[role="treeitem"]') ?? [])];
      const at = rows.findIndex(row => row.dataset.bin === (selected ?? ''));
      let next = selected;
      if (event.key === 'ArrowUp' || event.key === 'ArrowDown') next = rows[Math.max(0, Math.min(rows.length - 1, at + (event.key === 'ArrowUp' ? -1 : 1)))]?.dataset.bin || null;
      else if (event.key === 'ArrowRight') {
        if (selected && collapsed.has(selected)) toggle(selected);
        else next = bins.find(bin => bin.parent === selected)?.id ?? selected;
      } else if (event.key === 'ArrowLeft') {
        if (selected && !collapsed.has(selected) && bins.some(bin => bin.parent === selected)) toggle(selected);
        else next = bins.find(bin => bin.id === selected)?.parent ?? null;
      }
      onSelect(next);
      rows.find(row => row.dataset.bin === (next ?? ''))?.focus();
    }
  };
  const row = (id: string | null, name: string, level: number) => <div key={id ?? 'master'} className="pool-bin" role="treeitem" aria-level={level}
    aria-selected={id === selected} aria-expanded={bins.some(bin => bin.parent === id) ? id === null || !collapsed.has(id) : undefined}
    tabIndex={id === selected ? 0 : -1} data-bin={id ?? ''} style={{ paddingLeft: (level - 1) * 12 + 4 }}
    onClick={() => onSelect(id)} onDoubleClick={() => { if (id) toggle(id); }}
    onDragOver={event => { if (event.dataTransfer.types.includes(MEDIA_DRAG)) { event.preventDefault(); event.dataTransfer.dropEffect = 'move'; } }}
    onDrop={event => { event.preventDefault(); onMove(draggedMedia(event.dataTransfer, session), id); }}>
    <span className="pool-bin-expand" aria-hidden="true" onClick={event => { if (id) { event.stopPropagation(); toggle(id); } }}>
      {bins.some(bin => bin.parent === id) ? (id && collapsed.has(id) ? <ChevronRight size={12} /> : <ChevronDown size={12} />) : null}
    </span>
    <Folder size={14} aria-hidden="true" />
    <span className="pool-bin-name">{editing?.id === id && id !== null ? <NameField name={editing.name}
      onCommit={name => onEdit([{ op: 'set_bin', id, name }])} onCancel={() => setEditing(null)} /> : name}</span>
  </div>;
  const children = (parent: string | null, level: number): React.ReactNode => bins.filter(bin => bin.parent === parent).map(bin => <div key={bin.id}>
    {row(bin.id, bin.name, level)}
    {!collapsed.has(bin.id) && <div role="group">{children(bin.id, level + 1)}</div>}
  </div>);
  return <aside className="pool-bins" aria-label={words.bins}>
    <div className="pool-bin-tools">
      <button type="button" className="icon-btn" aria-label={words.newBin} title={words.newBin} onClick={() => {
        if (selected) setCollapsed(before => { const next = new Set(before); next.delete(selected); return next; });
        setEditing({ id: null, parent: selected, name: '' });
      }}><FolderPlus size={14} /></button>
      <button type="button" className="icon-btn" aria-label={words.rename} title={words.rename} disabled={selected === null} onClick={edit}><Pencil size={13} /></button>
      <button type="button" className="icon-btn" aria-label={words.removeBin} title={words.removeBin} disabled={selected === null} onClick={remove}><Trash2 size={13} /></button>
    </div>
    {editing?.id === null && <NameField name="" onCommit={name => onEdit([{ op: 'add_bin', bin: { id: '', name, parent: editing.parent } }])} onCancel={() => setEditing(null)} />}
    <div ref={tree} role="tree" aria-label={words.bins} onKeyDown={keys}>
      {row(null, words.master, 1)}
      <div role="group">{children(null, 2)}</div>
    </div>
  </aside>;
}
