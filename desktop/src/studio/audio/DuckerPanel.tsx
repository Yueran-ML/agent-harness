import { useEffectAutomation } from './useEffectAutomation';
import { FxMeter } from './FxMeter';
import { useEffect, useRef, useState } from 'react';
import type { Ducker } from '../engine';
import { useFailureText, useFairlightText } from '../fairlight/words';
import type { FairlightText } from '../../messages/fairlight';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { mixerStrips, stripChange, laneValue } from './mixerModel';
import { sidechainSources, targetKey } from './processorModel';
import { defaultDucker, DUCKER_NUMBERS } from './effectModel';
import type { ProcessorProps } from './useProcessor';
import { Knob } from './Knob';
export function DuckerPanel(props: ProcessorProps) {
  const t = useFairlightText(), text = (key: string) => t(key as FairlightText);
  const failure = useFailureText();
  const { timeline, target } = props, strip = mixerStrips(timeline).find(s => s.key === targetKey(target));
  const control = useEffectAutomation(props), { automation, recording } = control;
  const model = strip?.strip?.track_fx ?? defaultDucker();
  const disabled = !('track' in target) || !strip || strip.locked;
  const [problem, setProblem] = useState<string | null>(null), [preview, setPreview] = useState<Ducker | null>(null);
  const captured = useRef(model), touching = useRef<string | null>(null);
  const gesture = useAudioGesture({
    ...props, identity: `${timeline.id}/${targetKey(target)}/ducker`, locked: disabled, onFailure: error => setProblem(failure(error))
  });
  const path = (field: string) => `track_fx.${field}`;
  const touchActive = () => Boolean(touching.current && 'track' in target && automation?.active({ of: target, path: touching.current }));
  const activeField = useRef<string | null>(null);
  const active = (field?: string) => (field === undefined || activeField.current === field) && (touchActive() || gesture.active());
  const touchEnd = useRef<(() => void) | null>(null);
  const finishTouch = () => {
    touchEnd.current?.();
    touchEnd.current = null;
    touching.current = null;
  };
  useEffect(() => () => {
    finishTouch();
    activeField.current = null;
  }, [props.session, timeline.id, targetKey(target)]);
  const writable = (field: string) => !disabled && (recording ? control.supported(path(field)) : automation?.enabled === false || strip?.arm === 'off' || !strip?.lanes?.[path(field)]);
  const playback = { ...model };
  if (automation?.enabled !== false && strip?.arm !== 'off')
    for (const [key, lane] of Object.entries(strip?.lanes ?? {}))
      if (key.startsWith('track_fx.') && lane.points.length) {
        const field = key.slice(9), value = laneValue(lane, props.seconds ?? 0);
        (playback as unknown as Record<string, unknown>)[field] = field === 'on' ? value !== 0 : value;
      }
  const value = { ...(active() ? preview ?? playback : playback) };
  for (const live of automation?.enabled === false ? [] : automation?.lives ?? (automation?.live ? [automation.live] : [])) {
    if (!('track' in target) || !('track' in live.target.of) || live.target.of.track !== target.track || !live.target.path.startsWith('track_fx.'))
      continue;
    const field = live.target.path.slice(9);
    if (typeof (value as any)[field] === 'number' || typeof (value as any)[field] === 'boolean')
      (value as any)[field] = field === 'on' ? live.value >= .5 : live.value;
  }
  for (const field of Object.keys(value) as (keyof Ducker)[]) {
    const old = value[field];
    if (typeof old === 'number' || typeof old === 'boolean') {
      const read = control.reading(path(field), Number(old));
      (value as any)[field] = typeof old === 'boolean' ? read >= .5 : read;
    }
  }
  const ops = (settings: Partial<Ducker>) => strip ? [stripChange(timeline.id, strip, { strip: { track_fx: { ...model, ...settings } } })] : [];
  const commit = (field: string, next: unknown) => {
    if (!writable(field))
      return;
    if (recording) {
      if ('track' in target && (typeof next === 'number' || typeof next === 'boolean'))
        automation?.instant({ of: target, path: path(field) }, Number(next));
      return;
    }
    gesture.commit(ops({ [field]: next }));
  };
  if (!('track' in target))
    return null;
  return <section
    data-audio-owner={targetKey(target)}
    className="audio-ducker-panel"
    aria-label={t('Ducker')}
  >

    <h3>{t('Ducker')}</h3>
    {(problem || control.problem) && <p role="alert">{problem || control.problem}</p>}

    <FxMeter context={props} kind="ducker" />

    <label data-automation-state={control.status(path('on')) ?? undefined}>
      <input
        type="checkbox"
        checked={value.on}
        disabled={!writable('on')}
        onChange={event => commit('on', event.target.checked)}
      />
      {t('Enable')}
    </label>

    <button
      type="button"
      disabled={disabled || recording || automation?.enabled !== false && strip?.arm !== 'off' && Object.keys(strip?.lanes ?? {}).some(key => key.startsWith('track_fx.'))}
      onClick={() => gesture.commit(ops(defaultDucker()))}
    >
      {t('Reset')}
    </button>

    <label>
      {t('Source')}
      <select
        multiple
        aria-label={text('Ducker Sources')}
        value={value.sources.map(targetKey)}
        disabled={!writable('sources')}
        onChange={event => {
          const keys = [...event.target.selectedOptions].map(o => o.value);
          commit('sources', sidechainSources(timeline, target).filter(s => keys.includes(s.key)).map(s => s.source));
        }}
      >
        {sidechainSources(timeline, target).map(source => <option key={source.key} value={source.key} disabled={source.disabled}>{source.name}</option>)}
      </select>
    </label>

    <div className="processor-knobs">
      {DUCKER_NUMBERS.map(({ field, label, ...bounds }) => <div key={field} data-automation-state={control.status(path(field)) ?? undefined}>
        <Knob
          {...bounds}
          path={control.supported(path(field)) ? path(field) : undefined}
          label={text(label)}
          value={(value as any)[field]}
          disabled={!writable(field)}
          isActive={() => active(field)}
          onStart={() => {
            if (!writable(field))
              return false;
            if (recording) {
              finishTouch();
              activeField.current = null;
              if (!automation?.start({ of: target, path: path(field) }, (value as any)[field]))
                return false;
              touching.current = path(field);
              const control = automation;
              const owned = { of: target, path: path(field) };
              touchEnd.current = () => {
                if (control?.active(owned))
                  control.end();
              };
            }
            else if (!gesture.start(ops(model)))
              return false;
            activeField.current = field;
            captured.current = value;
            setPreview(value);
            return true;
          }}
          onChange={next => {
            if (!active(field))
              return;
            const changed = { ...captured.current, [field]: next };
            setPreview(changed);
            if (touchActive())
              automation?.change(next);
            else
              gesture.preview(ops(changed));
          }}
          onEnd={() => {
            if (activeField.current !== field)
              return;
            activeField.current = null;
            if (touching.current)
              finishTouch();
            else
              gesture.end();
            setPreview(null);
          }}
          onCancel={() => {
            if (activeField.current !== field)
              return;
            activeField.current = null;
            if (touching.current)
              finishTouch();
            else
              gesture.cancel();
            setPreview(null);
          }}
          onCommit={next => commit(field, next)}
        />

        {automation?.enabled && control.supported(path(field)) && <button
          type="button"
          aria-label={`${text('Automation')} ${text(label)}`}
          onClick={() => automation.show({ of: target, path: path(field) })}
        >◇</button>}
      </div>)}
    </div>
  </section>;
}
