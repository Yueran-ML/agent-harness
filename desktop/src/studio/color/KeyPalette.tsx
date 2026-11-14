import { RotateCcw } from 'lucide-react';
import { useWords } from '../../app/locale';
import { callEngine, type Corrector, type KeyInput, type KeyMixer, type KeyOutput } from '../engine';
import { NumberControl } from './controls';
import { shownValue, withValue } from './corrector';
import type { Edits, EditedNode, NodeEdit } from './useNodeEdit';
import './qualifier.css';

/**
 * The Key palette (Reference Manual 20, pp. 3358–3363; the interaction reference, §8.5): how the
 * key of the current node is taken in, made, and given on.
 *
 * - Of a corrector: **Key Input** (Input Invert, off and on by Add Outside, p. 3341; Matte, the
 *   default, which takes the key linked in with the node's own, or Mask, which adds them; Gain,
 *   Offset, Blur Radius and Blur H/V Ratio, p. 3362), **Key Output** (Output Invert, p. 3182; Gain,
 *   1.00 from 0 to 2.00, at 0 the node does nothing; Offset, the key's contrast, pp. 3362–3363),
 *   and **Qualifier** (Invert, Matte, which takes the qualifier with the windows, Gain, Offset,
 *   pp. 3199, 3362).
 * - Of a Key Mixer (pp. 3358–3360, 3363): a row of each input (Input Invert; Matte, which takes
 *   them together, or Mask, which adds them, the default; Gain; Offset), and its output.
 *
 * The manual gives no defaults but Matte and Output Gain: the others are the interaction
 * reference's (Gain 1.00, Offset 0, Blur Radius 0, H/V 0.50, both inverts off).
 */
const GAIN = { neutral: 1, low: 0, high: 2 };
const OFFSET = { neutral: 0, low: -1, high: 1 };
const BLUR = { neutral: 0, low: 0, high: 2000 };
const RATIO = { neutral: 0.5, low: 0, high: 1 };

type Group = 'input' | 'output' | 'qualifier';

/** Whether the Key palette of a corrector is changed from a new node's: its button shows the orange dot (p. 2976). */
export function keyChanged(corrector: Corrector): boolean {
  const key = corrector.key;
  const is = (param: unknown, neutral: number) => param === neutral;
  const plain = (input: KeyInput | undefined, matte: boolean) => !input
    || (!input.invert && input.matte === matte && is(input.gain, GAIN.neutral) && is(input.offset, OFFSET.neutral));
  return !plain(key.input, true) || !plain(key.qualifier, true) || key.output.invert || !is(key.output.gain, GAIN.neutral)
    || !is(key.output.offset, OFFSET.neutral) || (key.input_blur !== undefined && (!is(key.input_blur.radius, BLUR.neutral) || !is(key.input_blur.ratio, RATIO.neutral)));
}

/** A switch of the palette: on and off, or one of two names. */
function Toggle({ on, label, disabled, onChange, test }: { on: boolean; label: string; disabled: boolean; onChange: (on: boolean) => void; test: string }) {
  return <button type="button" className="color-tool" aria-pressed={on} disabled={disabled} onClick={() => onChange(!on)} data-test={test}>{label}</button>;
}

/** Matte and Mask, the one chosen pressed (p. 3362: a Matte takes the keys together, a Mask adds them). */
function MatteMask({ matte, disabled, onChange, test }: { matte: boolean; disabled: boolean; onChange: (matte: boolean) => void; test: string }) {
  const labels = useWords().label.studio.colorPage.keyPalette;
  return <span className="color-modes" role="radiogroup" aria-label={`${labels.matte} / ${labels.mask}`} data-test={test}>
    {([true, false] as const).map(each => <button key={String(each)} type="button" role="radio" aria-checked={matte === each} className="color-mode"
      disabled={disabled} onClick={() => { if (matte !== each) onChange(each); }}>{each ? labels.matte : labels.mask}</button>)}
  </span>;
}

/** The Key palette of a corrector, edited as the other palettes edit it. */
function CorrectorKey({ edit }: { edit: NodeEdit }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.keyPalette;
  const disabled = !edit.enabled;
  const key = (edit.corrector as Corrector).key;
  const qualifier = key.qualifier ?? { invert: false, matte: true, gain: 1, offset: 0 };
  const number = (path: string, name: string, bounds: { neutral: number; low: number; high: number }, digits = 2, perPx = 0.005) =>
    <NumberControl key={edit.controlKey([path])} name={name} field={path} value={edit.value(path)} neutral={bounds.neutral} low={bounds.low}
      high={bounds.high} digits={digits} perPx={perPx} keyed={edit.keyed(path)} keyedTitle={words.hint.studio.colorPage.keyed} disabled={disabled}
      onPreview={value => edit.preview([[path, value]])} onCommit={value => edit.commit([[path, value]])} onCancel={edit.cancel} />;
  const whole = (path: string, value: boolean): Edits => [[path, { whole: value }]];
  const reset = (group: Group) => {
    const at = `key.${group}`;
    const edits: Edits = [[`${at}.invert`, { whole: false }], [`${at}.gain`, GAIN.neutral], [`${at}.offset`, OFFSET.neutral]];
    const more: Edits = group === 'output' ? [] : group === 'qualifier' ? [[`${at}.matte`, { whole: true }]]
      : [[`${at}.matte`, { whole: true }], ['key.input_blur.radius', BLUR.neutral], ['key.input_blur.ratio', RATIO.neutral]];
    edit.commit([...edits, ...more]);
  };
  const head = (group: Group, title: string) => <div className="color-palette-head">
    <span className="overline">{title}</span>
    <span className="color-gap" />
    <button type="button" className="icon-btn" aria-label={`${labels.reset}: ${title}`} title={labels.reset} disabled={disabled}
      onClick={() => reset(group)} data-test={`reset-key-${group}`}><RotateCcw size={12} /></button>
  </div>;
  return <div className="color-key-groups">
    <section className="color-key-group" aria-label={labels.input} data-group="input">
      {head('input', labels.input)}
      <div className="color-settings">
        <Toggle on={key.input.invert} label={labels.invert} disabled={disabled} onChange={on => edit.commit(whole('key.input.invert', on))} test="key-input-invert" />
        <MatteMask matte={key.input.matte} disabled={disabled} onChange={matte => edit.commit(whole('key.input.matte', matte))} test="key-input-matte" />
        {number('key.input.gain', labels.gain, GAIN)}
        {number('key.input.offset', labels.offset, OFFSET)}
        {number('key.input_blur.radius', labels.blurRadius, BLUR, 1, 1)}
        {number('key.input_blur.ratio', labels.blurRatio, RATIO)}
      </div>
    </section>
    <section className="color-key-group" aria-label={labels.output} data-group="output">
      {head('output', labels.output)}
      <div className="color-settings">
        <Toggle on={key.output.invert} label={labels.invert} disabled={disabled} onChange={on => edit.commit(whole('key.output.invert', on))} test="key-output-invert" />
        {number('key.output.gain', labels.gain, GAIN)}
        {number('key.output.offset', labels.offset, OFFSET)}
      </div>
    </section>
    <section className="color-key-group" aria-label={labels.qualifier} data-group="qualifier">
      {head('qualifier', labels.qualifier)}
      <div className="color-settings">
        <Toggle on={qualifier.invert} label={labels.invert} disabled={disabled} onChange={on => edit.commit(whole('key.qualifier.invert', on))} test="key-qualifier-invert" />
        <MatteMask matte={qualifier.matte} disabled={disabled} onChange={matte => edit.commit(whole('key.qualifier.matte', matte))} test="key-qualifier-matte" />
        {number('key.qualifier.gain', labels.gain, GAIN)}
        {number('key.qualifier.offset', labels.offset, OFFSET)}
      </div>
    </section>
  </div>;
}

/**
 * The Key palette of a Key Mixer (pp. 3358–3360, 3363): its inputs and its output. A change is one
 * transaction of `set_node`, sent when it is let go; a setting with keys changes its key at the playhead.
 */
function MixerKey({ edited }: { edited: EditedNode }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.keyPalette;
  const node = edited.node;
  if (node.kind !== 'key_mixer') return null;
  const mixer: KeyMixer = node.params;
  const send = (params: Partial<KeyMixer>) => {
    void callEngine(edited.bridge, 'transaction.apply', {
      revision: edited.revision, ops: [{ op: 'set_node', grade: edited.target, id: node.id, params }],
    }, edited.session === undefined ? undefined : { expectedSession: edited.session }).catch(edited.onFailure);
  };
  const input = (index: number, change: Partial<KeyInput>) => send({ inputs: mixer.inputs.map((each, i) => (i === index ? { ...each, ...change } : each)) });
  const output = (change: Partial<KeyOutput>) => send({ output: { ...mixer.output, ...change } });
  // A setting with keys shows, and changes, the key at the playhead or to its left (p. 3391).
  const value = (param: KeyInput['gain']) => shownValue(param, edited.frame);
  const number = (field: string, name: string, now: number, bounds: typeof GAIN, onCommit: (value: number) => void) =>
    <NumberControl key={`${field}:${edited.revision}`} name={name} field={field} value={now} neutral={bounds.neutral} low={bounds.low} high={bounds.high}
      digits={2} perPx={0.005} onPreview={() => {}} onCommit={onCommit} onCancel={() => {}} />;
  return <div className="color-key-groups" data-test="key-mixer">
    {mixer.inputs.map((each, index) => <section key={index} className="color-key-group" aria-label={labels.mixerInput(index + 1)} data-group={`input-${index}`}>
      <span className="overline">{labels.mixerInput(index + 1)}</span>
      <div className="color-settings">
        <Toggle on={each.invert} label={labels.invert} disabled={false} onChange={on => input(index, { invert: on })} test={`mixer-invert-${index}`} />
        <MatteMask matte={each.matte} disabled={false} onChange={matte => input(index, { matte })} test={`mixer-matte-${index}`} />
        {number(`inputs.${index}.gain`, labels.gain, value(each.gain), GAIN, gain => input(index, { gain: withValue(each.gain, edited.frame, gain) }))}
        {number(`inputs.${index}.offset`, labels.offset, value(each.offset), OFFSET, offset => input(index, { offset: withValue(each.offset, edited.frame, offset) }))}
      </div>
    </section>)}
    <section className="color-key-group" aria-label={labels.output} data-group="output">
      <span className="overline">{labels.output}</span>
      <div className="color-settings">
        <Toggle on={mixer.output.invert} label={labels.invert} disabled={false} onChange={on => output({ invert: on })} test="mixer-output-invert" />
        {number('output.gain', labels.gain, value(mixer.output.gain), GAIN, gain => output({ gain: withValue(mixer.output.gain, edited.frame, gain) }))}
        {number('output.offset', labels.offset, value(mixer.output.offset), OFFSET, offset => output({ offset: withValue(mixer.output.offset, edited.frame, offset) }))}
      </div>
    </section>
  </div>;
}

export function KeyPalette({ edit, edited }: { edit: NodeEdit; edited: EditedNode | null }) {
  const said = useWords().hint.studio.colorPage;
  if (edited?.node.kind === 'key_mixer') return <div className="color-key" data-test="key">{<MixerKey edited={edited} />}</div>;
  if (!edit.enabled) return <p className="caption color-empty" data-test="no-corrector">{said.noKeyNode}</p>;
  return <div className="color-key" data-test="key"><CorrectorKey edit={edit} /></div>;
}
