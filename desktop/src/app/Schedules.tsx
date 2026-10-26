import { useEffect, useState } from 'react';
import { FolderOpen, Plus, Trash2 } from 'lucide-react';
import type { Workspace } from '../useWorkspace';
import { useWords } from './locale';
import {
  failureOf, schedulesOf, taskOf, WEEKDAYS, whenOf, type Every, type Schedule, type Weekday,
} from './work';

/**
 * The part of the settings for the tasks that run at set times: those there are, whether each
 * runs, how it ran last, and a form that adds one. Without the shell, as in a replay, there is
 * no part for it.
 */
export function Schedules({ workspace, language }: { workspace: Workspace; language: string }) {
  const words = useWords();
  const said = words.label.work.schedules;
  const told = words.hint.work.schedules;
  const bridge = window.columbina?.schedules ? window.columbina : undefined;
  const [tasks, setTasks] = useState<Schedule[] | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // A task runs in the folder of the project that is open, unless another one is chosen.
  const [cwd, setCwd] = useState(workspace.chat ? '' : workspace.folder ?? '');
  const [form, setForm] = useState({
    name: '', prompt: '', every: 'day' as Every, minutes: '30', time: '09:00', minute: '0',
    weekday: 'mon' as Weekday, missed: 'run' as 'run' | 'skip',
  });
  const task = taskOf({ ...form, cwd });

  const asked = async (what: () => Promise<unknown>) => {
    if (!bridge) return;
    setBusy(true);
    setFailed(null);
    try {
      await what();
      setTasks(schedulesOf(await bridge.schedules({ what: 'list' })));
      return true;
    } catch (problem) {
      setFailed(`${told.failed} ${failureOf(problem)}`);
      return false;
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    // Where the program cannot be asked, as where it is not there, the list stays away without a word.
    void bridge?.schedules({ what: 'list' }).then(answer => setTasks(schedulesOf(answer))).catch(() => {});
  }, []);
  // The part shows once the program has said which tasks there are.
  if (!bridge || tasks === null) return null;

  const add = async () => {
    if (task === null) return;
    if (await asked(() => bridge.schedules({ what: 'add', task }))) setForm(current => ({ ...current, name: '', prompt: '' }));
  };
  const choose = async () => {
    const chosen = await workspace.pickFolder(language);
    if (chosen !== null) setCwd(chosen);
  };
  const field = <K extends keyof typeof form>(key: K) => (value: typeof form[K]) => setForm(current => ({ ...current, [key]: value }));

  return <section className="setting" data-test="schedules">
    <h3>{said.title}</h3>
    <p className="caption">{told.about}</p>
    {tasks.length === 0 && <p className="caption" data-test="schedules-none">{told.none}</p>}
    {tasks.length > 0 && <ul className="work-list" data-test="schedules-list">
      {tasks.map(one => <li key={one.id} className="work-item" data-test="schedule">
        <div className="work-words">
          <strong>{one.name}</strong>
          <span className="caption">{whenOf(words, one)} · <span className="mono">{one.cwd}</span></span>
          {one.last !== null && one.lastEnd !== null && <span className="caption">
            {said.last(Intl.DateTimeFormat(language, { dateStyle: 'short', timeStyle: 'short' }).format(one.last), one.lastEnd)}
          </span>}
        </div>
        <label className="check">
          <input
            type="checkbox"
            checked={one.enabled}
            disabled={busy}
            onChange={event => void asked(() => bridge.schedules({ what: event.target.checked ? 'enable' : 'disable', id: one.id }))}
            data-test="schedule-on"
          />
          {said.on}
        </label>
        <button
          type="button"
          className="icon-btn"
          aria-label={said.remove}
          title={said.remove}
          disabled={busy}
          onClick={() => void asked(() => bridge.schedules({ what: 'remove', id: one.id }))}
          data-test="schedule-remove"
        >
          <Trash2 size={14} />
        </button>
      </li>)}
    </ul>}

    <div className="work-form" data-test="schedule-form">
      <div className="setting-row">
        <label className="setting-name" htmlFor="schedule-name">{said.name}</label>
        <input id="schedule-name" className="field setting-grow" value={form.name} maxLength={80} onChange={event => field('name')(event.target.value)} data-test="schedule-name" />
      </div>
      <div className="setting-row">
        <span className="setting-name">{said.folder}</span>
        <span className="mono setting-grow work-path" data-test="schedule-cwd">{cwd}</span>
        <button type="button" className="btn" onClick={() => void choose()} data-test="schedule-choose">
          <FolderOpen size={14} aria-hidden="true" />{said.chooseFolder}
        </button>
      </div>
      <label className="setting-name" htmlFor="schedule-prompt">{said.prompt}</label>
      <textarea id="schedule-prompt" className="field work-prompt" rows={3} value={form.prompt} onChange={event => field('prompt')(event.target.value)} data-test="schedule-prompt" />
      <div className="setting-row">
        <label className="setting-name" htmlFor="schedule-every">{said.every}</label>
        <select id="schedule-every" className="field" value={form.every} onChange={event => field('every')(event.target.value as Every)} data-test="schedule-every">
          {(['minutes', 'hour', 'day', 'week'] as const).map(kind => <option key={kind} value={kind}>{said.kinds[kind]}</option>)}
        </select>
        {form.every === 'minutes' && <input
          className="field work-number" type="number" min={5} aria-label={said.minutes} title={said.minutes}
          value={form.minutes} onChange={event => field('minutes')(event.target.value)} data-test="schedule-minutes"
        />}
        {form.every === 'hour' && <input
          className="field work-number" type="number" min={0} max={59} aria-label={said.minute} title={said.minute}
          value={form.minute} onChange={event => field('minute')(event.target.value)} data-test="schedule-minute"
        />}
        {form.every === 'week' && <select
          className="field" aria-label={said.weekday} value={form.weekday}
          onChange={event => field('weekday')(event.target.value as Weekday)} data-test="schedule-weekday"
        >
          {WEEKDAYS.map(day => <option key={day} value={day}>{said.days[day]}</option>)}
        </select>}
        {(form.every === 'day' || form.every === 'week') && <input
          className="field work-time" type="time" aria-label={said.at} value={form.time}
          onChange={event => field('time')(event.target.value)} data-test="schedule-time"
        />}
      </div>
      <div className="setting-row">
        <label className="setting-name" htmlFor="schedule-missed">{said.missed}</label>
        <select id="schedule-missed" className="field" value={form.missed} onChange={event => field('missed')(event.target.value as 'run' | 'skip')} data-test="schedule-missed">
          <option value="run">{said.missedKinds.run}</option>
          <option value="skip">{said.missedKinds.skip}</option>
        </select>
      </div>
      <div className="setting-row">
        <button type="button" className="btn" disabled={busy || task === null} onClick={() => void add()} data-test="schedule-add">
          <Plus size={14} aria-hidden="true" />{said.add}
        </button>
      </div>
    </div>
    {failed && <p className="caption" data-tone="warn" role="alert" data-test="schedules-failed">{failed}</p>}
  </section>;
}
