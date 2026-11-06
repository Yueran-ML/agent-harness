import type { Modifiers } from './mixerModel';
import { useEffect, useRef, useState } from 'react';
import { callEngine, type AudioPreset, type AudioPresetPayload } from '../engine';
import type { ProcessorProps } from './useProcessor';
import { cleanAudioPreset, factoryAudioPresets, presetKind } from './audioPresets';
import { ABCompare } from './ABCompare';
import { targetKey } from './processorModel';
import { useFairlightText } from '../fairlight/words';
import type { FairlightText } from '../../messages/fairlight';
export function EffectPresets({ context, identity, value, disabled, apply }: {
  context: ProcessorProps;
  identity: string;
  value: AudioPresetPayload;
  disabled: boolean;
  apply: (payload: AudioPresetPayload, modifiers?: Modifiers) => void;
}) {
  const t = useFairlightText(), [users, setUsers] = useState<AudioPreset[]>([]), [selected, setSelected] = useState('factory:0'), [name, setName] = useState(''), [naming, setNaming] = useState(false), [problem, setProblem] = useState<string | null>(null), [busy, setBusy] = useState(false);
  const scope = `${context.session}/${context.timeline.id}/${targetKey(context.target)}/${identity}`;
  const alive = useRef(true), generation = useRef(0);
  useEffect(() => {
    alive.current = true;
    generation.current++;
    setUsers([]);
    setSelected('factory:0');
    setBusy(false);
    setNaming(false);
    setProblem(null);
    return () => {
      alive.current = false;
      generation.current++;
    };
  }, [scope]);
  const load = async () => {
    const turn = generation.current;
    try {
      const result = await callEngine(context.bridge, 'audio_presets.list', {}, { expectedSession: context.session });
      if (alive.current && generation.current === turn)
        setUsers(result.presets);
    }
    catch (error) {
      if (alive.current && generation.current === turn)
        setProblem(String(error));
    }
  };
  const latestLoad = useRef(load);
  latestLoad.current = load;
  useEffect(() => context.bridge.onEvent?.(event => {
    if (event.kind === 'notification' && event.session === context.session && event.method === 'audio_presets.changed')
      void latestLoad.current();
  }), [context.bridge, context.session]);
  const presets = [...factoryAudioPresets(value), ...users.filter(p => presetKind(p.payload) === presetKind(value))];
  const choose = (id: string, modifiers: Modifiers = {}) => {
    const preset = presets.find(p => p.id === id);
    if (!preset || disabled)
      return;
    setSelected(id);
    apply(structuredClone(preset.payload), modifiers);
  };
  const save = async (update: boolean) => {
    if (busy || update && selected.startsWith('factory:'))
      return;
    const turn = generation.current;
    setBusy(true);
    setProblem(null);
    try {
      const payload = cleanAudioPreset(value);
      const result = update ? await callEngine(context.bridge, 'audio_presets.update', { preset: selected, payload }, { expectedSession: context.session }) : await callEngine(context.bridge, 'audio_presets.save', { name, payload }, { expectedSession: context.session });
      if (alive.current && generation.current === turn) {
        setSelected(result.id);
        setNaming(false);
        await load();
      }
    }
    catch (error) {
      if (alive.current && generation.current === turn)
        setProblem(String(error));
    }
    finally {
      if (alive.current && generation.current === turn)
        setBusy(false);
    }
  };
  return <div className="audio-presets">
    <label>
      {t('Preset')}
      <select
        aria-label={t('Preset')}
        disabled={disabled}
        value={selected}
        onFocus={() => void load()}
        onChange={event => choose(event.target.value, event.nativeEvent as MouseEvent)}
      >
        {presets.map(p => <option key={p.id} value={p.id}>{p.id.startsWith('factory:') ? t(p.name as FairlightText) : p.name}</option>)}
      </select>
    </label>

    <button
      type="button"
      aria-label={t('Previous preset')}
      disabled={disabled || presets.findIndex(p => p.id === selected) <= 0}
      onClick={() => choose(presets[presets.findIndex(p => p.id === selected) - 1]?.id)}
    >‹</button>
    <button
      type="button"
      aria-label={t('Next preset')}
      disabled={disabled || presets.findIndex(p => p.id === selected) >= presets.length - 1}
      onClick={() => choose(presets[presets.findIndex(p => p.id === selected) + 1]?.id)}
    >›</button>

    <button
      type="button"
      aria-label={t('Save preset')}
      disabled={busy}
      onClick={() => setNaming(old => !old)}
    >+</button>
    <button
      type="button"
      disabled={busy || selected.startsWith('factory:')}
      onClick={() => void save(true)}
    >
      {t('Update preset')}
    </button>

    <button
      type="button"
      disabled={busy || selected.startsWith('factory:')}
      onClick={async () => {
        const turn = generation.current;
        setBusy(true);
        try {
          await callEngine(context.bridge, 'audio_presets.delete', { preset: selected }, { expectedSession: context.session });
          if (alive.current && generation.current === turn) {
            setSelected('factory:0');
            await load();
          }
        }
        catch (error) {
          if (alive.current && generation.current === turn)
            setProblem(String(error));
        }
        finally {
          if (alive.current && generation.current === turn)
            setBusy(false);
        }
      }}
    >
      {t('Delete preset')}
    </button>

    {naming && <form onSubmit={event => {
      event.preventDefault();
      void save(false);
    }}>
      <input
        aria-label={t('Preset name')}
        maxLength={100}
        value={name}
        onChange={event => setName(event.target.value)}
      />
      <button type="submit" disabled={busy || !name.trim()}>{t('Save preset')}</button>
    </form>}

    <ABCompare
      bridge={context.bridge}
      identity={`${context.session}/${context.timeline.id}/${targetKey(context.target)}/${identity}`}
      value={value}
      disabled={disabled}
      apply={apply}
    />

    {problem && <p role="alert">{problem}</p>}
  </div>;
}
