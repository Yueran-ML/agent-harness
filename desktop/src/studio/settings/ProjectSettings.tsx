import { useCallback, useRef, useState, type ReactNode } from 'react';
import { RotateCw } from 'lucide-react';
import { useWords } from '../../app/locale';
import { Sheet } from '../../app/Sheet';
import type { EngineBridge } from '../../bridge';
import { callEngine, keptColor, type ColorSettings, type LutLibrary, type LutPair, type Project, type SatRolloff } from '../engine';
import { refusalText } from '../inspector/refusal';
import { namedLuts } from '../color/luts';
import { useLutLibrary } from '../color/useLuts';
import { hdrOffEverywhere, useViewerHdr } from '../viewerDisplay';
import {
  ACES_VERSIONS, acesVersionPatch, BYPASS, colorFieldsShown, DAVINCI, DRTS, GAMUT_NAMES, GAMUTS, gammasOf, LOG, LUMINANCE_NAMES, LUMINANCES,
  NODE_LUT_NAMES, ODTS, PRESET_NAMES, PRESETS, presetPatch, PROCESSING_NAMES, processingPatch, RESIZES, SCIENCE_NAMES, SCIENCES, separateGamma,
  SPACES, SPLIT_GAMUTS, splitSpace, joinSpace, withGamut, type ColorField,
} from './colorSettings';
import './settings.css';

type Panel = 'color' | 'general';
type Patch = Record<string, unknown>;

/**
 * The Project Settings window (Reference Manual 20, pp. 19–20): its panels in a sidebar at the left,
 * of which the studio has Color Management (pp. 149–153, 224–252) and the Color group of General
 * Options (p. 156). It opens from the gear at the lower right of every page.
 *
 * Resolve applies its settings when Save is pressed. Here each change is sent at once, as one
 * `set_color_settings` of what changed (and what has to change with it), which one undo takes back
 * in the one history of the project; what the engine refuses is said at the top of the panel.
 */
export function ProjectSettings({ bridge, project, onClose }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>;
  project: Project | null;
  onClose: () => void;
}) {
  const words = useWords();
  const labels = words.label.studio.settings;
  const said = words.hint.studio;
  const [panel, setPanel] = useState<Panel>('color');
  const [problem, setProblem] = useState<string | null>(null);
  const pending = useRef(false);
  const settings = project === null ? null : keptColor(project);

  const send = useCallback(async (patch: Patch) => {
    if (pending.current) { setProblem(said.edit.pending); return; }
    pending.current = true;
    setProblem(null);
    try {
      await callEngine(bridge, 'transaction.apply', { ops: [{ op: 'set_color_settings', settings: patch }] });
    } catch (error) {
      setProblem(refusalText(error, said));
    } finally {
      pending.current = false;
    }
  }, [bridge, said]);

  return <Sheet title={labels.title} wide onClose={onClose} foot={<button type="button" className="btn" onClick={onClose} data-first>{words.label.close}</button>}>
    <div className="project-settings" data-test="project-settings">
      <nav className="project-settings-panels" aria-label={labels.panels}>
        {(['color', 'general'] as const).map(each => <button key={each} type="button" aria-pressed={panel === each} onClick={() => setPanel(each)}
          data-test={`settings-panel-${each}`}>{each === 'color' ? labels.colorManagement : labels.generalOptions}</button>)}
      </nav>
      <div className="project-settings-panel">
        {problem && <p className="project-settings-problem" role="alert" data-test="settings-problem">{problem}</p>}
        {settings === null ? <p className="caption">{said.settings.noProject}</p> : panel === 'color'
          ? <ColorManagement bridge={bridge} settings={settings} send={send} />
          : <section aria-label={labels.color}>
            <h3 className="overline">{labels.color}</h3>
            <Check label={labels.sCurve} checked={settings.s_curve_contrast} onChange={on => void send({ s_curve_contrast: on })} test="s_curve_contrast" />
          </section>}
        <p className="caption">{said.settings.applied}</p>
      </div>
    </div>
  </Sheet>;
}

/** A row of the panel: its name, and what sets it. */
function Row({ label, htmlFor, children, test }: { label: string; htmlFor?: string; children: ReactNode; test: string }) {
  return <div className="project-setting" data-test={`setting-${test}`}>
    {htmlFor ? <label className="setting-name" htmlFor={htmlFor}>{label}</label> : <span className="setting-name">{label}</span>}
    <div className="project-setting-value">{children}</div>
  </div>;
}

function Check({ label, checked, onChange, test }: { label: string; checked: boolean; onChange: (on: boolean) => void; test: string }) {
  return <label className="project-setting project-check" data-test={`setting-${test}`}>
    <input type="checkbox" checked={checked} onChange={event => onChange(event.target.checked)} data-test={`color-${test}`} />
    <span>{label}</span>
  </label>;
}

function Choice<T extends string>({ id, label, value, options, onChange, test, aria }: {
  id: string; label?: string; value: T; options: readonly { value: T; label: string }[]; onChange: (value: T) => void; test: string; aria?: string;
}) {
  return <select id={id} className="field" aria-label={aria ?? label} value={value}
    onChange={(event) => { if (event.target.value !== value) onChange(event.target.value as T); }} data-test={`color-${test}`}>
    {options.map(each => <option key={each.value} value={each.value}>{each.label}</option>)}
  </select>;
}

/** A number of nits, sent when it is left or Enter is pressed; Escape puts it back. */
function Nits({ id, label, value, onCommit, test }: { id: string; label: string; value: number; onCommit: (value: number) => void; test: string }) {
  const [typed, setTyped] = useState<string | null>(null);
  const commit = () => {
    if (typed === null) return;
    const number = Number(typed);
    setTyped(null);
    if (typed.trim() !== '' && Number.isFinite(number) && number !== value) onCommit(number);
  };
  return <input id={id} className="field project-nits" type="number" aria-label={label} value={typed ?? String(value)} min={1} max={10000}
    onChange={event => setTyped(event.target.value)} onBlur={commit}
    onKeyDown={(event) => {
      if (event.key === 'Enter') { event.preventDefault(); commit(); }
      else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setTyped(null); }
    }} data-test={`color-${test}`} />;
}

/**
 * A space of the catalogue, by one menu, or by a gamut and a gamma where Use Separate Color Space
 * and Gamma is on (p. 234): a gamut chosen keeps the gamma where it has that one.
 */
function SpacePicker({ id, label, gammaLabel, value, bypass, split, onChange, test }: {
  id: string; label: string; gammaLabel: string; value: string; bypass: boolean; split: boolean; onChange: (space: string) => void; test: string;
}) {
  const labels = useWords().label.studio.settings;
  const bypassed = value === BYPASS;
  if (!split) {
    const options = [...(bypass ? [{ value: BYPASS, label: labels.bypass }] : []), ...SPACES.map(space => ({ value: space, label: space }))];
    return <Choice id={id} label={label} value={value} options={options} onChange={onChange} test={test} />;
  }
  const { gamut, gamma } = bypassed ? { gamut: BYPASS, gamma: '' } : splitSpace(value);
  const gamuts = [...(bypass ? [{ value: BYPASS, label: labels.bypass }] : []), ...SPLIT_GAMUTS.map(each => ({ value: each, label: each }))];
  return <>
    <Choice id={id} label={label} value={gamut} options={gamuts} onChange={next => onChange(next === BYPASS ? BYPASS : withGamut(value, next))} test={test} />
    <select className="field" aria-label={gammaLabel} value={gamma} disabled={bypassed} data-test={`color-${test}-gamma`}
      onChange={(event) => { const next = joinSpace(gamut, event.target.value); if (next !== null && next !== value) onChange(next); }}>
      {bypassed ? <option value="">{labels.bypass}</option> : gammasOf(gamut).map(each => <option key={each} value={each}>{each}</option>)}
    </select>
  </>;
}

/** The two menus of a stage of LUTs, its 1D and its 3D (pp. 151–152): no LUT, or one of the library, by the name a node gives it. */
function LutPairPicker({ field, label, value, library, send }: {
  field: 'input_lut' | 'output_lut' | 'viewer_lut'; label: string; value: LutPair; library: LutLibrary | null; send: (patch: Patch) => void;
}) {
  const labels = useWords().label.studio.settings;
  const names = library === null ? [] : namedLuts(library).map(lut => lut.name).sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }));
  const menu = (kind: '1d' | '3d') => {
    const chosen = value[kind];
    const options = [{ value: '', label: labels.noLut }, ...names.map(name => ({ value: name, label: name.replace(/\.cube$/i, '') }))];
    if (chosen !== null && !names.includes(chosen)) options.push({ value: chosen, label: `${chosen} (${labels.missing})` });
    return <Choice id={`${field}-${kind}`} aria={`${label}: ${kind === '1d' ? labels.lut1d : labels.lut3d}`} value={chosen ?? ''} options={options}
      onChange={name => send({ [field]: { [kind]: name === '' ? null : name } })} test={`${field}-${kind}`} />;
  };
  return <Row label={label} test={field}>{menu('1d')}{menu('3d')}</Row>;
}

/**
 * The Color Management panel (the interaction reference, §7.1, §5.3): Color Space & Transforms,
 * each setting shown where it applies (`colorFieldsShown`), and the Lookup Tables. Display HDR On
 * Viewers If Available is among them, as in Resolve, but is a preference of this computer.
 */
function ColorManagement({ bridge, settings, send }: { bridge: Pick<EngineBridge, 'call' | 'onEvent'>; settings: ColorSettings; send: (patch: Patch) => void }) {
  const words = useWords();
  const labels = words.label.studio.settings;
  const said = words.hint.studio.settings;
  const { library, refresh, loading } = useLutLibrary(bridge);
  const viewer = useViewerHdr(bridge);
  const shown = new Set<ColorField>(colorFieldsShown(settings));
  const split = separateGamma(settings);
  const set = (patch: Patch) => send(patch);
  const regrade = <p className="caption project-setting-hint">{said.regrade}</p>;
  const rolloff = (field: 'input_sat_rolloff' | 'output_sat_rolloff') => {
    const value: SatRolloff = settings[field];
    return <>
      <Row label={labels.rolloffStart} htmlFor={`${field}-start`} test={`${field}-start`}>
        <Nits id={`${field}-start`} label={labels.rolloffStart} value={value.start} onCommit={start => set({ [field]: { start } })} test={`${field}-start`} />
        <span className="caption">{labels.nits}</span>
      </Row>
      <Row label={labels.rolloffLimit} htmlFor={`${field}-limit`} test={`${field}-limit`}>
        <Nits id={`${field}-limit`} label={labels.rolloffLimit} value={value.limit} onCommit={limit => set({ [field]: { limit } })} test={`${field}-limit`} />
        <span className="caption">{labels.nits}</span>
      </Row>
    </>;
  };
  const drt = (field: 'input_drt' | 'output_drt', label: string) => <Row label={label} htmlFor={field} test={field}>
    <Choice id={field} label={label} value={settings[field]} test={field} onChange={value => set({ [field]: value })}
      options={DRTS.map(each => ({ value: each, label: each === 'davinci' ? DAVINCI : labels.drts[each] }))} />
  </Row>;

  return <div className="project-color" data-test="color-management">
    <section aria-label={labels.spaces}>
      <h3 className="overline">{labels.spaces}</h3>
      <Row label={labels.science} htmlFor="science" test="science">
        <Choice id="science" label={labels.science} value={settings.science} test="science" onChange={science => set({ science })}
          options={SCIENCES.map(each => ({ value: each, label: SCIENCE_NAMES[each] }))} />
      </Row>
      {shown.has('automatic') && <Check label={labels.automatic} checked={settings.automatic} onChange={automatic => set({ automatic })} test="automatic" />}
      {shown.has('processing_mode') && <Row label={labels.processingMode} htmlFor="processing_mode" test="processing_mode">
        <Choice id="processing_mode" label={labels.processingMode} value={settings.processing_mode} test="processing_mode" onChange={mode => set(processingPatch(mode))}
          options={(['sdr', 'hdr'] as const).map(each => ({ value: each, label: PROCESSING_NAMES[each] }))} />
      </Row>}
      {shown.has('preset') && <Row label={labels.preset} htmlFor="preset" test="preset">
        <Choice id="preset" label={labels.preset} value={settings.preset} test="preset" onChange={preset => set(presetPatch(preset))}
          options={PRESETS.map(each => ({ value: each, label: each === 'custom' ? labels.custom : PRESET_NAMES[each] }))} />
        {regrade}
      </Row>}
      {shown.has('output') && <Row label={labels.output} htmlFor="output" test="output">
        <SpacePicker id="output" label={labels.output} gammaLabel={labels.outputGamma} value={settings.output} bypass split={split} test="output"
          onChange={output => set({ output })} />
      </Row>}
      {shown.has('separate_gamma') && <Check label={labels.separateGamma} checked={settings.separate_gamma} onChange={on => set({ separate_gamma: on })} test="separate_gamma" />}
      {shown.has('input') && <Row label={labels.input} htmlFor="input" test="input">
        <SpacePicker id="input" label={labels.input} gammaLabel={labels.inputGamma} value={settings.input} bypass split={split} test="input"
          onChange={input => set({ input })} />
      </Row>}
      {shown.has('timeline') && <Row label={labels.timeline} htmlFor="timeline" test="timeline">
        <SpacePicker id="timeline" label={labels.timeline} gammaLabel={labels.timelineGamma} value={settings.timeline} bypass={false} split={split} test="timeline"
          onChange={timeline => set({ timeline })} />
        {regrade}
      </Row>}
      {shown.has('working_luminance') && <Row label={labels.workingLuminance} htmlFor="working_luminance" test="working_luminance">
        <Choice id="working_luminance" label={labels.workingLuminance} value={settings.working_luminance} test="working_luminance"
          onChange={value => set({ working_luminance: value })}
          options={LUMINANCES.map(each => ({ value: each, label: each === 'custom' ? labels.custom : LUMINANCE_NAMES[each] }))} />
        {shown.has('custom_luminance') && <>
          <Nits id="custom_luminance" label={`${labels.workingLuminance} (${labels.nits})`} value={settings.custom_luminance}
            onCommit={value => set({ custom_luminance: value })} test="custom_luminance" />
          <span className="caption">{labels.nits}</span>
        </>}
      </Row>}
      {shown.has('hdr_203_nits') && <Check label={labels.hdr203} checked={settings.hdr_203_nits} onChange={on => set({ hdr_203_nits: on })} test="hdr_203_nits" />}
      {shown.has('limit_gamut') && <Row label={labels.limitGamut} htmlFor="limit_gamut" test="limit_gamut">
        <Choice id="limit_gamut" label={labels.limitGamut} value={settings.limit_gamut ?? ''} test="limit_gamut"
          onChange={value => set({ limit_gamut: value === '' ? null : value })}
          options={[{ value: '', label: labels.none }, ...GAMUTS.map(each => ({ value: each, label: GAMUT_NAMES[each] }))]} />
      </Row>}
      {shown.has('input_drt') && drt('input_drt', labels.inputDrt)}
      {shown.has('input_sat_rolloff') && rolloff('input_sat_rolloff')}
      {shown.has('output_drt') && drt('output_drt', labels.outputDrt)}
      {shown.has('output_sat_rolloff') && rolloff('output_sat_rolloff')}
      {shown.has('inverse_drt') && <Check label={labels.inverseDrt} checked={settings.inverse_drt} onChange={on => set({ inverse_drt: on })} test="inverse_drt" />}
      {shown.has('white_point_adaptation') && <Check label={labels.whitePoint} checked={settings.white_point_adaptation}
        onChange={on => set({ white_point_adaptation: on })} test="white_point_adaptation" />}
      {shown.has('color_space_aware_tools') && <Check label={labels.aware} checked={settings.color_space_aware_tools}
        onChange={on => set({ color_space_aware_tools: on })} test="color_space_aware_tools" />}
      {shown.has('resize_in') && <Row label={labels.resizeIn} htmlFor="resize_in" test="resize_in">
        <Choice id="resize_in" label={labels.resizeIn} value={settings.resize_in} test="resize_in" onChange={value => set({ resize_in: value })}
          options={RESIZES.map(each => ({ value: each, label: each === 'log' ? LOG : labels.resizes[each] }))} />
      </Row>}
      {shown.has('graphics_white') && <Row label={labels.graphicsWhite} htmlFor="graphics_white" test="graphics_white">
        <Nits id="graphics_white" label={labels.graphicsWhite} value={settings.graphics_white} onCommit={value => set({ graphics_white: value })} test="graphics_white" />
        <span className="caption">{labels.nits}</span>
      </Row>}
      {shown.has('aces_version') && <Row label={labels.acesVersion} htmlFor="aces_version" test="aces_version">
        <Choice id="aces_version" label={labels.acesVersion} value={settings.aces_version} test="aces_version"
          onChange={version => set(acesVersionPatch(version, settings.odt))} options={ACES_VERSIONS.map(each => ({ value: each, label: `ACES ${each}` }))} />
      </Row>}
      {shown.has('idt') && <Row label={labels.idt} htmlFor="idt" test="idt">
        <Choice id="idt" label={labels.idt} value={settings.idt} test="idt" onChange={idt => set({ idt })} options={SPACES.map(each => ({ value: each, label: each }))} />
      </Row>}
      {shown.has('odt') && <Row label={labels.odt} htmlFor="odt" test="odt">
        <Choice id="odt" label={labels.odt} value={settings.odt} test="odt" onChange={odt => set({ odt })}
          options={ODTS[settings.aces_version].map(each => ({ value: each, label: each }))} />
      </Row>}
      {shown.has('gamut_compress') && <Check label={labels.gamutCompress} checked={settings.gamut_compress} onChange={on => set({ gamut_compress: on })} test="gamut_compress" />}
      {shown.has('node_luts_in') && <Row label={labels.nodeLuts} htmlFor="node_luts_in" test="node_luts_in">
        <Choice id="node_luts_in" label={labels.nodeLuts} value={settings.node_luts_in} test="node_luts_in" onChange={value => set({ node_luts_in: value })}
          options={(['timeline', 'ap0_linear'] as const).map(each => ({ value: each, label: NODE_LUT_NAMES[each] }))} />
      </Row>}
      <Check label={labels.displayHdr} checked={viewer.hdr} onChange={viewer.set} test="display_hdr" />
      <p className="caption project-setting-hint">{said.displayHdr}{viewer.hdr && hdrOffEverywhere(viewer.shows) ? ` ${said.hdrOff}` : ''}</p>
    </section>
    <section aria-label={labels.lookupTables}>
      <h3 className="overline">{labels.lookupTables}</h3>
      <LutPairPicker field="input_lut" label={labels.inputLut} value={settings.input_lut} library={library} send={send} />
      <LutPairPicker field="output_lut" label={labels.outputLut} value={settings.output_lut} library={library} send={send} />
      <LutPairPicker field="viewer_lut" label={labels.viewerLut} value={settings.viewer_lut} library={library} send={send} />
      <Row label={labels.interpolation} htmlFor="lut_interpolation" test="lut_interpolation">
        <Choice id="lut_interpolation" label={labels.interpolation} value={settings.lut_interpolation} test="lut_interpolation"
          onChange={value => set({ lut_interpolation: value })}
          options={[{ value: 'trilinear', label: labels.trilinear }, { value: 'tetrahedral', label: labels.tetrahedral }]} />
      </Row>
      <div className="project-setting">
        <button type="button" className="btn btn-quiet" onClick={refresh} disabled={loading} data-test="update-lists">
          <RotateCw size={13} aria-hidden="true" />{labels.updateLists}</button>
      </div>
      {library !== null && library.roots.length > 0 && <div className="caption project-lut-folders" data-test="lut-roots">
        <p>{said.lutFolders}</p>
        <ol>{library.roots.map(root => <li key={root.path} lang="en" data-exists={root.exists}>{root.path}</li>)}</ol>
      </div>}
    </section>
  </div>;
}
