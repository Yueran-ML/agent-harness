import { useContext, useState, type MouseEvent as ReactMouseEvent } from 'react';
import { ChevronDown, ChevronRight, Plus, Trash2 } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import type { Menu, MenuItem } from '../color/ColorMenu';
import { CoveringMenu } from '../color/CoveringMenu';
import type { Clip, Composition, FusionKind, Media, Op, Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { FusionInspector } from '../fusion/FusionInspector';
import { roomFor, shownComposition, type FusionAt } from '../fusion/edits';
import { namesOf } from '../fusion/graph';
import { clipLocations } from '../timeline/selection';
import { addEffect, chainEffects, effectsFor, enableEffect, hasMoreNodes, removeEffect } from './effectChain';
import { FieldsLocked } from './fields';
import type { Build, Commit } from './useCommit';

/**
 * The effects of a clip of picture, in the Effects tab of the Inspector of the Edit page (`clipEffects.ts`): Add Effect, a menu
 * of a Blur, a Mosaic and, for a clip of media, a 3D Keyer; and the list of the effects of the clip, in the order that its
 * picture goes through them, each with its switch (off, the node is passed through), the button that deletes it, and its
 * settings, which are those that the Inspector of the Fusion page has for its node, with their keys (`FusionInspector`).
 * The name of an effect opens and closes its settings.
 *
 * The effects are nodes of the composition of the clip. Its other nodes (a Merge, a mask, a Tracker) are not listed: where it has
 * any, the tab says that the Fusion page shows the whole, and, where the page can go there (`onFusion`), has a button that does.
 *
 * Each change is one transaction, worked out from the clip as the engine has it when it is sent: a clip that has no
 * composition is given the first one with the effect added to it. Effects are of one clip: with several chosen the tab says so.
 * On a locked track the effects are shown, and nothing changes them.
 */
export function ClipEffects({ clips, timeline, media, commit, bridge, session, revision, frame, onSeek, onFusion }: {
  /** The clips of picture chosen. */
  clips: readonly Clip[];
  timeline: Timeline; media: readonly Media[]; commit: Commit;
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number;
  /** The playhead, in frames of the timeline. */
  frame: number;
  onSeek: (frame: number, scrubbing: boolean) => void;
  /** Open the Fusion page on the one clip of picture chosen. */
  onFusion?: () => void;
}) {
  const words = useWords();
  const t = useFairlightText();
  const labels = words.label.studio.inspector;
  const said = words.hint.studio.inspector;
  const locked = useContext(FieldsLocked);
  const [menu, setMenu] = useState<Menu | null>(null);
  /** The effects whose settings are closed, by the ids of their nodes. */
  const [closed, setClosed] = useState<readonly string[]>([]);
  const clip = clips.length === 1 ? clips[0] : null;
  if (clip === null) return <p className="caption" data-test="effects-one">{said.effectsOne}</p>;

  const id = clip.id;
  const composition = shownComposition(clip);
  const effects = chainEffects(composition);
  const kinds: Record<FusionKind, string> = { ...words.label.studio.fusionPage.kinds, ...labels.effectKinds };
  const names = namesOf(composition, kinds);
  /** One transaction about the clip, worked out from it as it is when it is sent: nothing on a locked track, and nothing of a clip that is gone. */
  const build = (ops: (where: FusionAt, now: Clip, shown: Composition) => Op[]): Build => (current) => {
    const located = clipLocations(current).get(id);
    if (!located || located.track.locked || located.track.kind !== 'video') return [];
    return ops({ timeline: current.id, clip: id }, located.clip, shownComposition(located.clip));
  };
  const openMenu = (event: ReactMouseEvent<HTMLButtonElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    const items: MenuItem[] = effectsFor(clip).map(kind => ({
      id: `effect-${kind}`, label: labels.effectKinds[kind],
      run: () => void commit(build((where, now) => addEffect(where, now, kind))),
    }));
    setMenu({ x: box.left, y: box.bottom, label: t('Add effect'), items });
  };

  return <div className="clip-effects" data-test="clip-effects" data-clip={id}>
    <div className="inspector-section-row">
      <h3 className="inspector-section">{labels.video}</h3>
      <button type="button" className="btn btn-quiet clip-effects-add" aria-haspopup="menu" disabled={locked || !roomFor(composition, 1)}
        onClick={openMenu} data-test="add-clip-effect"><Plus size={13} aria-hidden="true" />{t('Add effect')}</button>
    </div>
    {effects.length === 0 && <p className="caption" data-test="effects-none">{said.noEffects}</p>}
    {effects.length > 0 && <ol className="clip-effect-list" data-test="clip-effect-list">
      {effects.map((node) => {
        const name = names.get(node.id) ?? node.id;
        const on = !(node.pass_through ?? false);
        const open = !closed.includes(node.id);
        return <li key={node.id} className="clip-effect" data-effect={node.id} data-kind={node.kind} data-enabled={on || undefined}>
          <div className="clip-effect-head">
            <input type="checkbox" checked={on} disabled={locked} aria-label={`${t('Enable')}: ${name}`} title={t('Enable')} data-test="effect-enabled"
              onChange={(event) => {
                const enabled = event.target.checked;
                void commit(build((where, now) => enableEffect(where, now, node.id, enabled)));
              }} />
            <button type="button" className="clip-effect-name" aria-expanded={open} data-test="effect-name"
              onClick={() => setClosed(was => (open ? [...was, node.id] : was.filter(each => each !== node.id)))}>
              {open ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}
              <span>{name}</span>
            </button>
            <button type="button" className="icon-btn" aria-label={`${t('Delete effect')}: ${name}`} title={t('Delete effect')} disabled={locked}
              data-test="remove-effect" onClick={() => void commit(build((where, now) => removeEffect(where, now, node.id)))}>
              <Trash2 size={13} aria-hidden="true" /></button>
          </div>
          {open && <FusionInspector key={`${id}/${node.id}`} embedded bridge={bridge} session={session} revision={revision} timeline={timeline} clip={clip}
            media={media} node={node} name={name} frame={frame} onSeek={onSeek} locked={locked} />}
        </li>;
      })}
    </ol>}
    {hasMoreNodes(composition) && <div className="clip-effects-more" data-test="effects-composition">
      <p className="caption">{said.effectsComposition}</p>
      {onFusion && <button type="button" className="btn btn-quiet" onClick={onFusion} data-test="open-fusion">{labels.openInFusion}</button>}
    </div>}
    {menu && <CoveringMenu menu={menu} onClose={() => setMenu(null)} />}
  </div>;
}
