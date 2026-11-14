import { memo, type Dispatch, type SetStateAction } from 'react';
import { LocateFixed, Maximize2, Minus, Plus, MousePointer2, Scissors, Magnet, Link, Spline, UnfoldHorizontal } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Tool } from './interact';

/**
 * The bar above the timeline: its name, the tools, snapping, linked selection, zoom and following
 * the playhead. It is drawn again only when one of these changes, not as the timeline scrolls or
 * plays: every call it is given stays the same.
 *
 * Where a page has a Curve Editor (`onCurves`), its button is here too, lit while it is open, as
 * the Keyframes and Curve buttons of DaVinci Resolve's timeline (Reference Manual 20, pp. 1108, 1115).
 */
export const Toolbar = memo(function Toolbar({ name, tool, snapping, linked, follow, onTool, onSnapping, onLinked, onZoom, onFit, onFollow, onEditMenu, hideFollow = false, curves = false, onCurves }: {
  name: string; tool: Tool; snapping: boolean; linked: boolean; follow: boolean;
  onTool: (tool: Tool) => void; onSnapping: Dispatch<SetStateAction<boolean>>; onLinked: Dispatch<SetStateAction<boolean>>;
  onZoom: (factor: number) => void; onFit: () => void; onFollow: Dispatch<SetStateAction<boolean>>;
  hideFollow?: boolean;
  onEditMenu?: (anchor: HTMLElement) => void;
  /** Whether the Curve Editor of the page is open, and what opens and closes it. */
  curves?: boolean; onCurves?: () => void;
}) {
  const words = useWords();
  const labels = words.label.studio.timeline;
  return <header className="timeline-toolbar">
    <strong>{name}</strong>
    {onEditMenu && <button type="button" aria-haspopup="menu" onClick={event => onEditMenu(event.currentTarget)}>{words.label.studio.page.edit}</button>}
    {([['select', 'A', MousePointer2], ['trim', 'T', UnfoldHorizontal], ['blade', 'B', Scissors]] as const).map(([each, key, Icon]) => <button key={each} type="button" className="icon-btn"
      aria-label={labels[each]} title={`${labels[each]} (${key})`} aria-pressed={tool === each} onClick={() => onTool(each)}><Icon size={14} /></button>)}
    <button type="button" className="icon-btn" aria-label={labels.snap} title={`${labels.snap} (N)`} aria-pressed={snapping} onClick={() => onSnapping(value => !value)}><Magnet size={14} /></button>
    <button type="button" className="icon-btn" aria-label={labels.linked} title={`${labels.linked} (Ctrl+Shift+L)`} aria-pressed={linked} onClick={() => onLinked(value => !value)}><Link size={14} /></button>
    <button type="button" className="icon-btn" aria-label={labels.zoomOut} title={`${labels.zoomOut} (Ctrl+−)`} onClick={() => onZoom(0.8)}><Minus size={14} /></button>
    <button type="button" className="icon-btn" aria-label={labels.zoomIn} title={`${labels.zoomIn} (Ctrl++)`} onClick={() => onZoom(1.25)}><Plus size={14} /></button>
    <button type="button" className="icon-btn" aria-label={labels.fit} title={`${labels.fit} (Shift+Z)`} onClick={onFit}><Maximize2 size={14} /></button>
    {!hideFollow && <button type="button" className="icon-btn" aria-label={labels.follow} title={labels.follow} aria-pressed={follow} onClick={() => onFollow(value => !value)}><LocateFixed size={14} /></button>}
    {onCurves && <button type="button" className="icon-btn" aria-label={words.label.studio.curves.title} title={words.label.studio.curves.title} aria-pressed={curves}
      onClick={onCurves} data-test="toggle-curves"><Spline size={14} /></button>}
  </header>;
});
