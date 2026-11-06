import { useEffectAutomation } from '../audio/useEffectAutomation';
import { laneValue } from '../audio/mixerModel';
import { useEffect, useState } from 'react';
import type { Effect, InsertsOf } from '../engine';
import type { ProcessorProps } from '../audio/useProcessor';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { addEffect, defaultEffect, effectOwner, insertDrop, insertParameters } from '../audio/effectModel';
import { EffectPanel } from '../audio/EffectPanel';
import { useEffectFavorites } from '../audio/effectFavorites';
import { targetKey } from '../audio/processorModel';
import { operationTargets, mixerStrips } from '../audio/mixerModel';
import { AUDIO_EFFECTS, AUDIO_EFFECT_DRAG } from './EffectsLibrary';
import { useFailureText, useFairlightText } from './words';
import type { FairlightText } from '../../messages/fairlight';
import { chainClipboard as sharedChain } from '../audio/audioClipboard';
import { EffectWindow } from './EffectWindow';
import { clearAudioComparisons } from '../audio/ABCompare';
export const INSERT_DRAG = 'application/x-columbina-audio-insert';
const GROUPS: [
  string,
  Effect['kind'][]
][] = [['Dynamics', ['gain', 'limiter', 'soft_clipper']], ['Repair', ['de_esser', 'de_hummer', 'noise_reduction']], ['Spatial / Pitch', ['stereo_width', 'pitch']]];
export function InsertSlots({ inline = false, ...props }: ProcessorProps & {
  inline?: boolean;
}) {
  const t = useFairlightText(), text = (key: string) => t(key as FairlightText), [favorites] = useEffectFavorites();
  const failure = useFailureText();
  const owner = effectOwner(props.timeline, props.target), [open, setOpen] = useState<string | null>(null), [collapsed, setCollapsed] = useState<string[]>([]), [problem, setProblem] = useState<string | null>(null);
  const control = useEffectAutomation(props), { automation, recording } = control;
  const key = targetKey(props.target), chainClipboard = sharedChain.use();
  const [menuOpen, setMenuOpen] = useState(false);
  const gesture = useAudioGesture({
    ...props, identity: `${props.timeline.id}/${key}/inserts`, locked: owner.locked || recording, onFailure: error => setProblem(failure(error))
  });
  const writable = !owner.locked && !recording;
  const identity = (i: number) => owner.inserts[i]?.id || `${i}/${owner.inserts[i]?.effect.kind}`;
  useEffect(() => {
    const prefix = `${props.session}/${props.timeline.id}/${key}/insert/`;
    clearAudioComparisons(props.bridge, prefix, owner.inserts.map((insert, i) => `${prefix}${insert.id || i}/${insert.effect.kind}`));
  }, [props.bridge, props.session, props.timeline.id, key, owner.inserts]);
  const switchPath = (i: number) => 'insert.' + owner.inserts[i].id + '.on';
  const switchValue = (i: number) => {
    const insert = owner.inserts[i], lane = owner.lanes[switchPath(i)];
    const base = automation?.enabled !== false && owner.arm !== 'off' && lane?.points.length ? laneValue(lane, props.seconds ?? 0) : Number(insert.on);
    return control.reading(switchPath(i), base) >= .5;
  };
  const switchWritable = (i: number) => !owner.locked && (recording ? control.supported(switchPath(i)) : automation?.enabled === false || owner.arm === 'off' || !owner.lanes[switchPath(i)]);
  const toggle = (i: number) => {
    if (!switchWritable(i))
      return;
    const on = !switchValue(i);
    if (recording) {
      if (!('clip' in props.target))
        automation?.instant({ of: props.target, path: switchPath(i) }, Number(on));
      return;
    }
    gesture.commit(insertParameters(props.timeline, props.target, i, { ...owner.inserts[i], on }));
  };
  const name = (kind: Effect['kind']) => t(AUDIO_EFFECTS.find(([k]) => k === kind)![1]);
  const add = (kind: Effect['kind'], event: {
    altKey?: boolean;
    ctrlKey?: boolean;
  }) => {
    if (!writable)
      return;
    const targets: InsertsOf[] = 'clip' in props.target ? [props.target] : operationTargets(mixerStrips(props.timeline), key, props.selected ?? [], event).map(s => s.kind === 'track' ? { track: s.id } : { bus: s.id });
    gesture.commit(targets.flatMap(of => addEffect(props.timeline, of, kind)));
  };
  const drop = (event: React.DragEvent, index: number) => {
    if (!writable)
      return;
    const fx = event.dataTransfer.getData(AUDIO_EFFECT_DRAG), raw = event.dataTransfer.getData(INSERT_DRAG);
    if (AUDIO_EFFECTS.some(([kind]) => kind === fx)) {
      event.preventDefault();
      event.stopPropagation();
      add(fx as Effect['kind'], event);
    }
    if (!raw)
      return;
    event.preventDefault();
    event.stopPropagation();
    try {
      const source = JSON.parse(raw);
      if (source.session !== props.session || source.timeline !== props.timeline.id)
        return;
      gesture.commit(insertDrop(props.timeline, source.of, source.index, props.target, index, event.altKey));
    }
    catch { /* Foreign drag data is ignored. */
    }
  };
  const accept = (event: React.DragEvent) => {
    if (writable && [INSERT_DRAG, AUDIO_EFFECT_DRAG].some(type => event.dataTransfer.types.includes(type))) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
  return <div
    data-audio-owner={key}
    className="insert-slots"
    aria-label={t('Effects')}
    onDragOver={accept}
    onDrop={event => drop(event, owner.inserts.length)}
  >

    {(problem || control.problem) && <p role="alert">{problem || control.problem}</p>}

    {!inline && owner.inserts.map((insert, index) => <div
      key={identity(index)}
      className="insert-slot"
      draggable={writable}
      onDragStart={event => {
        event.stopPropagation();
        event.dataTransfer.setData(INSERT_DRAG, JSON.stringify({
          session: props.session, timeline: props.timeline.id, of: props.target, index
        }));
      }}
      onDragOver={accept}
      onDrop={event => drop(event, index)}
    >

      <button
        type="button"
        aria-label={`${t('Enable')} ${name(insert.effect.kind)}`}
        aria-pressed={switchValue(index)}
        data-automation-state={control.status(switchPath(index)) ?? undefined}
        disabled={!switchWritable(index)}
        onClick={() => toggle(index)}
      >●</button>

      <button type="button" onClick={() => setOpen(identity(index))}>{name(insert.effect.kind)}</button>

      <select
        aria-label={`${text('Replace effect')} ${index + 1}`}
        value=""
        disabled={!writable}
        onChange={event => gesture.commit([{
          edit: 'set_insert', timeline: props.timeline.id, of: props.target, slot: index, insert: { on: true, effect: defaultEffect(event.target.value as Effect['kind']) }
        }])}
      >
        <option value="">↔</option>
        {AUDIO_EFFECTS.map(([kind]) => <option key={kind} value={kind}>{name(kind)}</option>)}
      </select>

      <button
        type="button"
        aria-label={`${text('Delete effect')} ${index + 1}`}
        disabled={!writable}
        onClick={() => gesture.commit([{
          edit: 'set_insert', timeline: props.timeline.id, of: props.target, slot: index, insert: null
        }])}
      >×</button>
    </div>)}

    <button
      type="button"
      aria-label={text('Add effect')}
      aria-expanded={menuOpen}
      disabled={!writable || !('clip' in props.target) && owner.inserts.length >= 6}
      onClick={() => setMenuOpen(old => !old)}
    >+</button>
    {menuOpen && <div role="menu" className="insert-add-menu">

      {favorites.length > 0 && <fieldset>
        <legend>{t('Favorite')}</legend>
        {favorites.map(kind => <button
          type="button"
          key={kind}
          disabled={!writable || !('clip' in props.target) && owner.inserts.length >= 6}
          onClick={event => add(kind, event)}
        >
          {name(kind)}
        </button>)}
      </fieldset>}

      {GROUPS.map(([group, kinds]) => <fieldset key={group}>
        <legend>{text(group)}</legend>
        {kinds.map(kind => <button
          type="button"
          key={kind}
          disabled={!writable || !('clip' in props.target) && owner.inserts.length >= 6}
          onClick={event => add(kind, event)}
        >
          {name(kind)}
        </button>)}
      </fieldset>)}
    </div>}

    <div className="insert-chain-actions">
      <button type="button" onClick={() => {
        sharedChain.set({
          bridge: props.bridge, session: props.session, timeline: props.timeline.id, of: props.target
        });
      }}>
        {text('Copy effects')}
      </button>
      <button
        type="button"
        disabled={!writable || !chainClipboard || chainClipboard.bridge !== props.bridge || chainClipboard.session !== props.session || chainClipboard.timeline !== props.timeline.id}
        onClick={() => {
          if (chainClipboard)
            gesture.commit([{
              edit: 'copy_effects', timeline: props.timeline.id, from: chainClipboard.of, to: [props.target]
            }]);
        }}
      >
        {text('Paste effects')}
      </button>
    </div>

    {owner.inserts.map((insert, index) => {
      if (!inline && open !== identity(index))
        return null;
      const panel = <EffectPanel
        {...props}
        index={index}
        expanded={!collapsed.includes(identity(index))}
        onExpand={all => setCollapsed(old => all ? old.length ? [] : owner.inserts.map((_, i) => identity(i)) : old.includes(identity(index)) ? old.filter(x => x !== identity(index)) : [...old, identity(index)])}
        onMove={writable ? delta => gesture.commit([{
          edit: 'move_insert', timeline: props.timeline.id, of: props.target, from: index, to: index + delta
        }]) : undefined}
        onDelete={writable ? () => gesture.commit([{
          edit: 'set_insert', timeline: props.timeline.id, of: props.target, slot: index, insert: null
        }]) : undefined}
      />;
      return inline ? <div key={identity(index)}>{panel}</div> : <EffectWindow
        key={identity(index)}
        title={name(insert.effect.kind)}
        close={() => setOpen(null)}
      >{panel}</EffectWindow>;
    })}
  </div>;
}
