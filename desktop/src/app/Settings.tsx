import { useEffect, useRef, useState } from 'react';
import { FileText, KeyRound } from 'lucide-react';
import { MusicSettings, type BackgroundMusic } from '../BackgroundMusic';
import { called, changedBeyondKey } from '../key/form';
import type { Hello } from '../protocol';
import { sizes, type SizeId } from '../size';
import { themes, type ThemeId } from '../themes';
import type { Workspace } from '../useWorkspace';
import { sayConfinement, sayPermissions } from '../words';
import { Sheet } from './Sheet';
import { languages, useWords, type LanguageChoice } from './locale';
import { anotherServiceChosen, presetOf, presets, servicesToTell, serviceTitle } from './tell';

/** The part of the settings that a way into them leads to. */
export type SettingsPart = 'appearance' | 'model' | 'permissions';

export function Settings({
  workspace, part, theme, onTheme, size, onSize, language, onLanguage, shownLanguage, playOpening, onPlayOpening, onPlayNow,
  tellMe, onTellMe, music, onCommand, onClose,
}: {
  workspace: Workspace;
  part: SettingsPart;
  theme: ThemeId;
  onTheme: (theme: ThemeId) => void;
  size: SizeId;
  onSize: (size: SizeId) => void;
  language: LanguageChoice;
  onLanguage: (language: LanguageChoice) => void;
  /** The language of the words that are shown, as a page names it. */
  shownLanguage: string;
  playOpening: boolean;
  onPlayOpening: (play: boolean) => void;
  onPlayNow: () => void;
  tellMe: boolean;
  onTellMe: (tell: boolean) => void;
  music: BackgroundMusic;
  /** Give a command to the service. A change that widens what the task may do is confirmed first. */
  onCommand: (line: string) => void;
  onClose: () => void;
}) {
  const words = useWords();
  const { status, hello } = workspace;
  const running = status?.status === 'running';
  const commands = hello?.commands ?? [];
  const off = running || workspace.loading || !workspace.connected || workspace.sending || status?.status === 'stopped';
  const canModel = commands.some(command => command.name === 'model');
  const canPermissions = commands.some(command => command.name === 'permissions');

  const [model, setModel] = useState(status?.model.model ?? '');
  useEffect(() => setModel(status?.model.model ?? ''), [status?.model.model]);
  const services = workspace.services;
  // The greeting of the service process that ran when something was stored that it does not
  // know of: another address, model or window for its service. One that is started later does.
  // A key alone is no such thing: the process looks for the key with every request.
  const [storedUnder, setStoredUnder] = useState<Hello | null | undefined>(undefined);
  // What was stored is in use once the service process is started again: it talks to another
  // service than the one that is chosen, or more than a key was stored for its own.
  const stored = (storedUnder !== undefined && storedUnder === hello)
    || anotherServiceChosen(services, status?.model.provider);
  const [settingUp, setSettingUp] = useState(false);
  const setUp = async (service: string | null) => {
    setSettingUp(true);
    try {
      const before = services;
      const after = await workspace.setUpService(service, shownLanguage);
      if (after !== null && changedBeyondKey(before, after.listed, inUse)) setStoredUnder(hello);
    } finally {
      setSettingUp(false);
    }
  };
  const { inUse, known } = servicesToTell(services, status?.model.provider);
  const keyFrom = (service: { key: string; key_env: string | null }) => (service.key_env === null ? words.label.key.from.notNeeded
    : service.key === 'environment' ? words.label.key.from.environment
      : service.key === 'stored' ? words.label.key.from.stored
        : service.key === 'stored-for-another-address' ? words.label.key.from.storedElsewhere
          : words.label.key.from.none);

  // The settings open at the part that was asked for.
  const start = useRef<HTMLElement>(null);
  useEffect(() => {
    start.current?.scrollIntoView({ block: 'start' });
  }, []);
  const at = (name: SettingsPart) => (part === name ? start : undefined);

  const mode = status?.sandbox?.mode ?? null;
  const policy = status?.approval ?? null;
  const current = presetOf(mode, policy);
  const confinement = status?.sandbox ? sayConfinement(words, status.sandbox.strength) : null;

  return <Sheet
    title={words.label.settings}
    onClose={onClose}
    wide
    foot={<button type="button" className="btn btn-main" onClick={onClose}>{words.label.done}</button>}
  >
    <section className="setting" ref={at('appearance')}>
      <h3>{words.label.appearance}</h3>
      <div className="setting-row">
        <span className="setting-name" id="setting-theme">{words.label.theme}</span>
        {/* Buttons that are pressed or not: they need no keys beyond those of a button. */}
        <div className="segments" role="group" aria-labelledby="setting-theme">
          {themes.map(id => <button
            type="button"
            key={id}
            aria-pressed={theme === id}
            onClick={() => onTheme(id)}
            data-test={`theme-${id}`}
          >
            <span className="swatch" data-swatch={id} aria-hidden="true" />{words.label.themeNames[id]}
          </button>)}
        </div>
      </div>
      <div className="setting-row">
        <label className="setting-name" htmlFor="setting-language">{words.label.language}</label>
        <select
          id="setting-language"
          className="field"
          value={language}
          onChange={event => onLanguage(event.target.value as LanguageChoice)}
          data-test="language"
        >
          <option value="system">{words.label.languageSystem}</option>
          {languages.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
        </select>
      </div>
      <div className="setting-row">
        <span className="setting-name" id="setting-size">{words.label.typeSize}</span>
        <div className="segments" role="group" aria-labelledby="setting-size">
          {sizes.map(id => <button
            type="button"
            key={id}
            aria-pressed={size === id}
            onClick={() => onSize(id)}
            data-test={`size-${id}`}
          >
            {words.label.typeSizes[id]}
          </button>)}
        </div>
      </div>
    </section>

    <section className="setting" ref={at('model')}>
      <h3 id="setting-model-title">{words.label.model}</h3>
      {running && <p className="caption">{words.hint.settingsWhenIdle}</p>}
      <form
        className="setting-row"
        onSubmit={event => {
          event.preventDefault();
          if (model.trim()) onCommand(`/model ${model.trim()}`);
        }}
      >
        <input
          aria-labelledby="setting-model-title"
          className="field mono"
          value={model}
          onChange={event => setModel(event.target.value)}
          disabled={off || !canModel}
          spellCheck={false}
        />
        <button type="submit" className="btn" disabled={off || !canModel || !model.trim() || model.trim() === status?.model.model}>
          {words.label.apply}
        </button>
      </form>
      {status && <p className="caption" data-test="model-service">{serviceTitle(status.model.provider, services)}</p>}

      <div className="overline services-title">{words.label.key.title}</div>
      {known.length > 0 && <ul className="services" data-test="services">
        {known.map(service => <li key={service.name}>
          <span className="service-name">{called(service)}</span>
          {service.name === inUse && <span className="count">{words.label.key.inUse}</span>}
          {/* A service of the user's own is known by its name, so where it is stands beside it. */}
          {service.own && <span className="caption mono service-at" title={service.base_url}>{service.base_url}</span>}
          <span className="caption service-key" data-key={service.key}>{keyFrom(service)}</span>
        </li>)}
      </ul>}
      {services === null && <p className="caption">{words.hint.key.unavailable}</p>}
      <div className="setting-row">
        <button
          type="button"
          className="btn"
          disabled={services === null || settingUp}
          onClick={() => void setUp(inUse)}
          data-test="set-up-service"
        >
          <KeyRound size={14} aria-hidden="true" />{words.label.key.setUp}
        </button>
      </div>
      {stored && <div className="setting-row" data-test="stored-note">
        <p className="caption setting-grow">{words.hint.key.restart}</p>
        <button type="button" className="btn" disabled={running || workspace.loading} onClick={() => void workspace.reconnect()}>
          {words.label.restart}
        </button>
      </div>}
    </section>

    <section className="setting" ref={at('permissions')}>
      <h3>{words.label.permissions}</h3>
      {running && <p className="caption">{words.hint.settingsWhenIdle}</p>}
      {/* What holds now, also where it is none of the three below. */}
      {status && !current && <p className="setting-now">{sayPermissions(words, mode, policy)}</p>}
      <div className="choices" role="group" aria-label={words.label.permissions}>
        {presets.map(preset => <button
          type="button"
          key={preset.id}
          className="choice"
          aria-pressed={current === preset.id}
          disabled={off || !canPermissions}
          data-tone={preset.id === 'full-access' ? 'warn' : undefined}
          onClick={() => { if (current !== preset.id) onCommand(`/permissions ${preset.id}`); }}
          data-test={`preset-${preset.id}`}
        >
          <span className="choice-mark" aria-hidden="true" />
          <span className="choice-words">
            <strong>{words.label.presets[preset.name]}</strong>
            <span>{sayPermissions(words, preset.mode, preset.policy)}</span>
          </span>
        </button>)}
      </div>
      {confinement && <p className="caption">{confinement}</p>}
    </section>

    <section className="setting">
      <h3>{words.label.openingAndMusic}</h3>
      <div className="setting-row">
        <label className="check">
          <input type="checkbox" checked={playOpening} onChange={event => onPlayOpening(event.target.checked)} data-test="play-opening" />
          {words.label.playOpening}
        </label>
        <button type="button" className="btn" disabled={running || workspace.loading || workspace.approval !== null} onClick={onPlayNow}>
          {words.label.playOpeningNow}
        </button>
      </div>
      <MusicSettings music={music} />
    </section>

    <section className="setting">
      <h3>{words.label.notices}</h3>
      <div className="setting-row">
        <label className="check">
          <input type="checkbox" checked={tellMe} onChange={event => onTellMe(event.target.checked)} data-test="tell-me" />
          {words.label.tellMe}
        </label>
      </div>
      <p className="caption">{words.hint.tellMe}</p>
    </section>

    <section className="setting" data-test="about">
      <h3>{words.label.about}</h3>
      {hello && <p className="setting-now">{words.hint.about(hello.server.version, hello.protocol)}</p>}
      <p className="caption">
        {!workspace.connected ? words.hint.serviceEnded
          : workspace.mode === 'mock' ? words.hint.replay : words.hint.connectedService}
      </p>
      {hello && <p className="caption mono setting-path">{hello.cwd}</p>}
      <div className="setting-row">
        <button type="button" className="btn" disabled={running || workspace.loading} onClick={() => void workspace.reconnect()}>
          {words.label.restart}
        </button>
        {/* Where the service process writes what goes wrong. The shell shows the file in its folder. */}
        {window.columbina && <button type="button" className="btn btn-quiet" onClick={workspace.showLog} data-test="about-log">
          <FileText size={14} aria-hidden="true" />{words.label.showLog}
        </button>}
      </div>
      {window.columbina && <p className="caption">{words.hint.logKept}</p>}
    </section>
  </Sheet>;
}
