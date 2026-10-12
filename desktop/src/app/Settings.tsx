import { useEffect, useRef, useState } from 'react';
import { MusicSettings, type BackgroundMusic } from '../BackgroundMusic';
import { themes, type ThemeId } from '../themes';
import type { Workspace } from '../useWorkspace';
import { sayConfinement, sayPermissions } from '../words';
import { Sheet } from './Sheet';
import { languages, useWords, type LanguageChoice } from './locale';
import { presetOf, presets } from './tell';

/** The part of the settings that a way into them leads to. */
export type SettingsPart = 'appearance' | 'model' | 'permissions';

export function Settings({
  workspace, part, theme, onTheme, language, onLanguage, playOpening, onPlayOpening, onPlayNow, tellMe, onTellMe,
  music, onCommand, onClose,
}: {
  workspace: Workspace;
  part: SettingsPart;
  theme: ThemeId;
  onTheme: (theme: ThemeId) => void;
  language: LanguageChoice;
  onLanguage: (language: LanguageChoice) => void;
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
    foot={<>
      <span className="caption">{hello ? words.hint.about(hello.server.version, hello.protocol) : ''}</span>
      <button type="button" className="btn btn-main" onClick={onClose}>{words.label.done}</button>
    </>}
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
      {status && <p className="caption">{status.model.provider}</p>}
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
          <input type="checkbox" checked={playOpening} onChange={event => onPlayOpening(event.target.checked)} />
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

    <section className="setting">
      <h3>{words.label.connection}</h3>
      <p className="setting-now">
        {!workspace.connected ? words.hint.serviceEnded
          : workspace.mode === 'mock' ? words.hint.replay : words.hint.connectedService}
      </p>
      {hello && <p className="caption mono setting-path">{hello.cwd}</p>}
      <div className="setting-row">
        <button type="button" className="btn" disabled={running || workspace.loading} onClick={() => void workspace.reconnect()}>
          {words.label.restart}
        </button>
      </div>
    </section>
  </Sheet>;
}
