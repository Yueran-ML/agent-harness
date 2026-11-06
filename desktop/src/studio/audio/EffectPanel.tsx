import { effectClipboard } from './audioClipboard';
import { cleanAudioPreset } from './audioPresets';
import type { Effect } from '../engine';
import type { FairlightText } from '../../messages/fairlight';
import { AUDIO_EFFECTS } from '../fairlight/EffectsLibrary';
import { useFairlightText } from '../fairlight/words';
import { Knob } from './Knob';
import { defaultEffect, EFFECT_NUMBERS } from './effectModel';
import { sidechainSources, targetKey } from './processorModel';
import type { ProcessorProps } from './useProcessor';
import { useEffectEditor } from './useEffectEditor';
import { EffectDiagram } from './EffectDiagram';
import { FxMeter } from './FxMeter';
import { NoiseLearn } from './NoiseLearn';
import { EffectPresets } from './EffectPresets';
export function EffectPanel({ index, expanded = true, onExpand, onMove, onDelete, ...props }: ProcessorProps & {
  index: number;
  expanded?: boolean;
  onExpand?: (all: boolean) => void;
  onMove?: (delta: number) => void;
  onDelete?: () => void;
}) {
  const t = useFairlightText(), editor = useEffectEditor(props, index), clipboard = effectClipboard.use();
  const insert = editor.value;
  if (!insert)
    return null;
  const effect = insert.effect, mono = effect.kind === 'stereo_width' && editor.owner.channels === 1;
  const voice = effect.kind === 'noise_reduction' && effect.mode === 'voice';
  const text = (key: string) => t(key as FairlightText);
  const toggle = (field: string, label: string, value: boolean, disabled = false) => <label data-automation-state={editor.status(field) ?? undefined}>
    <input
      type="checkbox"
      checked={value}
      disabled={disabled || !editor.writable(field)}
      onChange={event => editor.field(field, event.target.checked, event.nativeEvent as MouseEvent)}
    />
    {text(label)}
  </label>;
  const choices = (field: string, label: string, value: string, values: string[]) => <label>
    {text(label)}
    <select
      aria-label={text(label)}
      value={value}
      disabled={editor.recording || !editor.writable(field)}
      onChange={event => editor.field(field, event.target.value)}
    >
      {values.map(option => <option key={option} value={option}>{text(option)}</option>)}
    </select>
  </label>;
  return <section
    data-audio-owner={targetKey(props.target)}
    className="audio-effect-panel"
    aria-label={text(AUDIO_EFFECTS.find(([kind]) => kind === effect.kind)![1])}
  >

    <header onDoubleClick={event => {
      if (!(event.target instanceof HTMLInputElement) && !(event.target instanceof HTMLButtonElement))
        onExpand?.(event.altKey);
    }}>

      {toggle('on', 'Enable', insert.on)}
      <strong>{text(AUDIO_EFFECTS.find(([kind]) => kind === effect.kind)![1])}</strong>

      {onMove && <>
        <button
          type="button"
          aria-label={text('Move effect up')}
          disabled={index === 0 || editor.owner.locked}
          onClick={() => onMove(-1)}
        >↑</button>
        <button
          type="button"
          aria-label={text('Move effect down')}
          disabled={index === editor.owner.inserts.length - 1 || editor.owner.locked}
          onClick={() => onMove(1)}
        >↓</button>
      </>}

      {onDelete && <button
        type="button"
        aria-label={text('Delete effect')}
        disabled={editor.owner.locked}
        onClick={onDelete}
      >×</button>}
    </header>

    {editor.problem && <p role="alert">{editor.problem}</p>}

    {expanded && <>
      <div className="processor-toolbar">
        <button type="button" onClick={() => {
          const clean = cleanAudioPreset({ kind: 'effect', settings: effect });
          if (clean.kind === 'effect')
            effectClipboard.set(clean.settings);
        }}>
          {t('Copy')}
        </button>
        <button
          type="button"
          disabled={!editor.writable() || clipboard?.kind !== effect.kind}
          onClick={event => {
            if (clipboard)
              editor.replace(structuredClone(clipboard), event);
          }}
        >
          {t('Paste')}
        </button>
        <button
          type="button"
          disabled={!editor.writable()}
          onClick={event => editor.replace(defaultEffect(effect.kind), event)}
        >
          {t('Reset')}
        </button>
      </div>

      <EffectPresets
        context={props}
        identity={`insert/${insert.id || index}/${effect.kind}`}
        value={{ kind: 'effect', settings: effect }}
        disabled={!editor.writable()}
        apply={(payload, modifiers) => {
          if (payload.kind === 'effect')
            editor.replace(payload.settings, modifiers);
        }}
      />

      {['soft_clipper', 'de_hummer', 'de_esser'].includes(effect.kind) && <EffectDiagram context={props} slot={index} effect={effect} />}

      {['limiter', 'de_esser', 'noise_reduction', 'stereo_width'].includes(effect.kind) && !mono && <FxMeter
        context={props}
        kind={effect.kind as 'limiter' | 'de_esser' | 'noise_reduction' | 'stereo_width'}
        insert={insert.id}
        slot={index}
        voice={voice}
      />}

      {effect.kind === 'noise_reduction' && <NoiseLearn
        context={props}
        slot={index}
        insert={insert.id}
        disabled={voice || !editor.writable() || Boolean(editor.recording)}
      />}

      {mono && <p>{text('Unavailable on mono')}</p>}

      <div className="processor-knobs">
        {EFFECT_NUMBERS[effect.kind].map(parameter => {
          const { field, label, ...bounds } = parameter, disabled = mono || (voice && !['dry_wet', 'level_db'].includes(field)) || !editor.writable(field);
          const normalize = (value: number) => parameter.integer ? Math.round(value) : value;
          return <div key={field} data-automation-state={editor.status(field) ?? undefined}>
            <Knob
              {...bounds}
              label={text(label)}
              path={editor.supported(field) ? editor.path(field) : undefined}
              value={(effect as unknown as Record<string, number>)[field]}
              disabled={disabled}
              onStart={() => editor.start(field)}
              onChange={value => editor.change(field, normalize(value))}
              onEnd={() => editor.end(field)}
              onCancel={() => editor.cancel(field)}
              onCommit={value => editor.field(field, normalize(value))}
              isActive={() => editor.active(field)}
            />

            {editor.automation?.enabled && editor.supported(field) && !('clip' in props.target) && <button
              type="button"
              aria-label={`${text('Automation')} ${text(label)}`}
              onClick={() => editor.show(field)}
            >◇</button>}

            <button
              type="button"
              aria-label={`${t('Reset')} ${text(label)}`}
              disabled={disabled}
              onClick={() => editor.field(field, parameter.fallback)}
            >↺</button>
          </div>;
        })}
      </div>

      {effect.kind === 'limiter' && <>
        {toggle('soft', 'Soft', effect.soft)}
        {toggle('sidechain.on', 'Sidechain', effect.sidechain.on)}
        {toggle('sidechain.listen', 'Listen', effect.sidechain.listen)}

        {!('clip' in props.target) && <label>
          {t('Source')}
          <select
            aria-label={t('Sidechain Source')}
            value={effect.sidechain.source ? targetKey(effect.sidechain.source) : ''}
            disabled={!editor.writable('sidechain.source')}
            onChange={event => editor.field('sidechain.source', sidechainSources(props.timeline, props.target as never).find(s => s.key === event.target.value)?.source ?? null)}
          >
            <option value="">{t('None')}</option>
            {sidechainSources(props.timeline, props.target).map(s => <option key={s.key} value={s.key} disabled={s.disabled}>{s.name}</option>)}
          </select>
        </label>}
      </>}

      {effect.kind === 'de_esser' && <>
        {choices('range', 'Range', effect.range, ['narrow', 'wide', 'all_high'])}
        {choices('reaction', 'Reaction', effect.reaction, ['slow', 'medium', 'fast'])}
        {toggle('listen', 'Listen', effect.listen)}
      </>}

      {effect.kind === 'de_hummer' && <>
        {[50, 60].map(hz => <button
          key={hz}
          type="button"
          disabled={!editor.writable('frequency_hz')}
          aria-pressed={effect.frequency_hz === hz}
          onClick={() => editor.field('frequency_hz', hz)}
        >
          {hz} Hz</button>)}
        {toggle('listen', 'Listen', effect.listen)}
      </>}

      {effect.kind === 'noise_reduction' && <>
        {choices('mode', 'Mode', effect.mode, ['auto_speech', 'manual', 'voice'])}
        {toggle('time_smoothing', 'Time Smoothing', effect.time_smoothing, voice)}
        {toggle('listen', 'Listen', effect.listen, voice)}
      </>}
    </>}
  </section>;
}
