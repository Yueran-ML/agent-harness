import { memo, type Dispatch, type SetStateAction } from 'react';
import { LocateFixed, Maximize2, Minus, Plus, MousePointer2, Scissors, Magnet, Link, UnfoldHorizontal } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Tool } from './interact';

/**
 * The bar above the timeline: its name, the tools, snapping, linked selection, zoom and following
 * the playhead. It is drawn again only when one of these changes, not as the timeline scrolls or
 * plays: every call it is given stays the same.
 */
export const Toolbar = memo(function Toolbar({ name, tool, snapping, linked, follow, onTool, onSnapping, onLinked, onZoom, onFit, onFollow, hideFollow = false }: {
  name: string; tool: Tool; snapping: boolean; linked: boolean; follow: boolean;
  onTool: (tool: Tool) => void; onSnapping: Dispatch<SetStateAction<boolean>>; onLinked: Dispatch<SetStateAction<boolean>>;
  onZoom: (factor: number) => void; onFit: () => void; onFollow: Dispatch<SetStateAction<boolean>>;
  hideFollow?: boolean;
}) {
  const labels = useWords().label.studio.timeline;
  return <header className="timeline-toolbar">
    <strong>{name}</strong>
    {([['select', 'A', MousePointer2], ['trim', 'T', UnfoldHorizontal], ['blade', 'B', Scissors]] as const).map(([each, key, Icon]) => <button key={each} type="button" className="icon-btn"
      aria-label={labels[each]} title={`${labels[each]} (${key})`} aria-pressed={tool === each} onClick={() => onTool(each)}><Icon size={14} /></button>)}
    <button type="button" className="icon-btn" aria-label={labels.snap} title={`${labels.snap} (N)`} aria-pressed={snapping} onClick={() => onSnapping(value => !value)}><Magnet size={14} /></button>
    <button type="button" className="icon-btn" aria-label={labels.linked} title={`${labels.linked} (Ctrl+Shift+L)`} aria-pressed={linked} onClick={() => onLinked(value => !value)}><Link size={14} /></button>
    <button type="button" className="icon-btn" aria-label={labels.zoomOut} title={`${labels.zoomOut} (Ctrl+−)`} onClick={() => onZoom(0.8)}><Minus size={14} /></button>
    <button type="button" className="icon-btn" aria-label={labels.zoomIn} title={`${labels.zoomIn} (Ctrl++)`} onClick={() => onZoom(1.25)}><Plus size={14} /></button>
    <button type="button" className="icon-btn" aria-label={labels.fit} title={`${labels.fit} (Shift+Z)`} onClick={onFit}><Maximize2 size={14} /></button>
    {!hideFollow && <button type="button" className="icon-btn" aria-label={labels.follow} title={labels.follow} aria-pressed={follow} onClick={() => onFollow(value => !value)}><LocateFixed size={14} /></button>}
  </header>;
});
