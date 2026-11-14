/**
 * What the Color page does with grades besides the node editor (the interaction reference, §2.7
 * and §9.4): undo and redo in the one history (Ctrl+Z, Ctrl+Shift+Z); the grade of the clip one or
 * two before (= and -, p. 3288); whole grades copied and pasted where the thumbnail timeline has
 * the focus (Ctrl+C, Ctrl+V, p. 3286); and the right-click menu of a thumbnail, with Collapse Group
 * Grades (pp. 3306–3307). It also keeps, for the page, how grades are sent and what the node
 * editor's view remembers.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from 'react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import type { Clip, Grade, Target, Timeline } from '../engine';
import { editingAllowed, inStudio } from '../keys';
import { ColorMenu, type Menu, type MenuItem } from './ColorMenu';
import { NodeViews } from './NodeEditor';
import type { GradeLevel } from './clips';
import { applyPrior, collapseGroups, defaultNodeOf, pageKey, pasteGrade, targetOf, withDefaultNode, type DefaultNode } from './nodeEdits';
import { useGradeEditing, type GradeEditing } from './useGradeEditing';

export interface ColorPageGrades {
  /** The grade that the node editor shows, as the engine names it; null where there is none. */
  target: Target | null;
  /**
   * The default node 01 that the grade is shown with where it has no node (pp. 3309, 3312), which the
   * first change to it makes (`onDefaultNode`); null where it has nodes.
   */
  defaultNode: DefaultNode | null;
  /** The grade as the node editor and the palettes show it: with its default node where it has none. */
  shown: Grade | null;
  editing: GradeEditing;
  views: NodeViews;
  /** The right-click of a thumbnail. */
  onThumbnailMenu: (event: ReactMouseEvent) => void;
  /** The menu that is open, to be drawn by the page. */
  menu: ReactNode;
}

export function useColorPageGrades({ bridge, revision, session, area, level, timeline, clips, current, chosen, grade = null, nextId, thumbnailItems }: {
  bridge: Pick<EngineBridge, 'call'>;
  revision: number;
  session: number;
  /** The part of the page that has the focus (p. 2967). */
  area: string;
  /** The level of the grade that the node editor shows. */
  level: GradeLevel;
  timeline: Timeline | null;
  /** The clips of the thumbnail timeline, in its order. */
  clips: readonly Clip[];
  current: Clip | null;
  /** The clips chosen in the thumbnail timeline. */
  chosen: ReadonlySet<string>;
  /** The grade of the level shown, as the project has it. */
  grade?: Grade | null;
  /** The project's `next_id`, which names the default node that a grade without nodes is shown with. */
  nextId?: number;
  /** More items of a thumbnail's menu, for the clip right-clicked: Wipe Timeline Clip (T27). */
  thumbnailItems?: (clip: string) => MenuItem[];
}): ColorPageGrades {
  const labels = useWords().label.studio.colorPage.nodeMenu;
  const target = targetOf(level, timeline, current);
  const stable = useMemo(() => target, [JSON.stringify(target)]);
  const found = defaultNodeOf(grade, stable, nextId);
  const defaultNode = useMemo(() => found, [JSON.stringify(found)]);
  const shown = useMemo(() => (grade === null ? null : withDefaultNode(grade, defaultNode?.id ?? null)), [grade, defaultNode]);
  const editing = useGradeEditing({ bridge, revision, session, defaultNode });
  const views = useMemo(() => new NodeViews(), [session]);
  const copied = useRef<Target | null>(null);
  const [menu, setMenu] = useState<Menu | null>(null);
  useEffect(() => { copied.current = null; setMenu(null); }, [session, timeline?.id]);

  const now = useRef({ editing, area, timeline, clips, current, chosen, thumbnailItems });
  now.current = { editing, area, timeline, clips, current, chosen, thumbnailItems };
  /** The clips that a command of the thumbnail timeline is for: those chosen, where `clip` is one of them, else it alone. */
  const chosenWith = (clip: string): Clip[] => {
    const { clips: all, chosen: picked } = now.current;
    return picked.has(clip) ? all.filter(each => picked.has(each.id)) : all.filter(each => each.id === clip);
  };

  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key)) return;
      const command = pageKey(key);
      if (command === null) return;
      const state = now.current;
      if (command.kind === 'history') { event.preventDefault(); state.editing.step(command.method); return; }
      if (state.timeline === null) return;
      if (command.kind === 'prior') {
        event.preventDefault();
        state.editing.run(applyPrior(state.timeline.id, state.clips, state.current?.id ?? null, command.back));
        return;
      }
      // Ctrl+C and Ctrl+V copy and paste whole grades where the thumbnail timeline has the focus (p. 3286).
      if (state.area !== 'thumbnails' || state.current === null) return;
      event.preventDefault();
      if (command.kind === 'copy') { copied.current = { level: 'clip', timeline: state.timeline.id, clip: state.current.id }; return; }
      if (copied.current === null) return;
      const to = (state.chosen.has(state.current.id) ? chosenWith(state.current.id) : [state.current])
        .map((clip): Target => ({ level: 'clip', timeline: state.timeline!.id, clip: clip.id }));
      state.editing.run(pasteGrade(copied.current, to));
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, []);

  const onThumbnailMenu = useCallback((event: ReactMouseEvent) => {
    const clip = (event.target as Element).closest<HTMLElement>('[data-clip]')?.dataset.clip;
    const timelineId = now.current.timeline?.id;
    if (!clip || !timelineId) return;
    event.preventDefault();
    const of = chosenWith(clip);
    const collapse = collapseGroups(timelineId, of);
    setMenu({ x: event.clientX, y: event.clientY, label: labels.thumbMenu, items: [
      ...(now.current.thumbnailItems?.(clip) ?? []),
      { id: 'collapse', label: labels.collapse, disabled: collapse === null, run: () => now.current.editing.run(collapse) },
    ] });
  }, [labels]);

  const close = useCallback(() => setMenu(null), []);
  return { target: stable, defaultNode, shown, editing, views, onThumbnailMenu, menu: menu && <ColorMenu menu={menu} onClose={close} /> };
}
