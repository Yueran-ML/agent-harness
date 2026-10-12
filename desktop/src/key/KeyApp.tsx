import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import { Check, KeyRound, TriangleAlert } from 'lucide-react';
import { Words } from '../app/locale';
import { messagesFor } from '../i18n';
import { themes } from '../themes';
import {
  chosen, complete, documentOf, formFor, listedOf, sentenceOf, serviceOf, storedOf, takesKey, tokensOf,
  type Form, type Listed, type Stored,
} from './form';

/** The value that the list of services has for "add one of your own". No service can be at that place. */
const ADD = 'add';

/**
 * The window in which a service is chosen and its key is given.
 *
 * It shows nothing that a model wrote, and it is the only place of the
 * application that ever has a key. What is typed goes to the shell, which
 * hands it to the program that stores it, and the answer holds no key.
 */
export function KeyApp() {
  const [language, setLanguage] = useState('en');
  const words = useMemo(() => messagesFor(language), [language]);
  const [listed, setListed] = useState<Listed | null>(null);
  // What went wrong before there was a form: a sentence, or an empty text for nothing known.
  const [failed, setFailed] = useState<string | null>(null);
  const [form, setForm] = useState<Form | null>(null);
  const [storing, setStoring] = useState(false);
  const [answer, setAnswer] = useState<Stored | null>(null);
  const first = useRef<HTMLSelectElement>(null);
  const keyField = useRef<HTMLInputElement>(null);
  const done = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let here = true;
    void (async () => {
      try {
        const context = await window.columbinaKey!.context();
        if (!here) return;
        setLanguage(context.language);
        document.documentElement.lang = context.language;
        if (themes.some(theme => theme === context.theme)) document.documentElement.dataset.theme = context.theme;
        const read = listedOf(context.list);
        if (read === null) {
          const said = (context.list as { error?: unknown } | null)?.error;
          setFailed(typeof said === 'string' ? said : '');
          return;
        }
        setListed(read);
        setForm(formFor(read, context.service));
      } catch (problem) {
        if (here) setFailed(problem instanceof Error ? problem.message : String(problem));
      }
    })();
    return () => { here = false; };
  }, []);

  // The keyboard begins where the user most often has something to do: at the key, if the
  // service is set up already, and at the service otherwise.
  const begun = useRef(false);
  useEffect(() => {
    if (begun.current || form === null || listed === null) return;
    begun.current = true;
    const service = serviceOf(form, listed);
    if (service !== null && service.model !== null && takesKey(form, listed)) keyField.current?.focus();
    else first.current?.focus();
  }, [form, listed]);
  useEffect(() => { if (answer?.kind === 'stored') done.current?.focus(); }, [answer]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !event.isComposing && !storing) {
        event.preventDefault();
        void window.columbinaKey?.close();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [storing]);

  const close = () => void window.columbinaKey?.close();
  const change = (given: Partial<Form>) => {
    setAnswer(null);
    setForm(current => (current === null ? current : { ...current, ...given }));
  };
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (form === null || listed === null || storing || !complete(form)) return;
    setStoring(true);
    setAnswer(null);
    try {
      const told = storedOf(await window.columbinaKey!.store(documentOf(form, listed)));
      setAnswer(told);
      // Once it is stored, the key has no business in this window any more.
      if (told.kind === 'stored') setForm(current => (current === null ? current : { ...current, key: '' }));
    } catch (problem) {
      setAnswer({ kind: 'refused', said: problem instanceof Error ? problem.message : String(problem) });
    } finally {
      setStoring(false);
    }
  };

  const service = form !== null && listed !== null ? serviceOf(form, listed) : null;
  const keyNote = service === null ? null
    : service.key_env === null ? words.hint.key.notNeeded
      : service.key === 'environment' ? words.hint.key.fromVariable(service.key_env)
        : service.key === 'stored' ? words.hint.key.stored
          : service.key === 'stored-for-another-address' ? words.hint.key.storedElsewhere
            : words.hint.key.none;
  const badWindow = form !== null && Number.isNaN(tokensOf(form.window));

  return <Words.Provider value={words}>
    <main className="key-page" data-test="key-page">
      <header className="key-head">
        <KeyRound size={18} aria-hidden="true" />
        <h1 className="display">{words.label.key.title}</h1>
      </header>
      <p className="key-lead">{words.hint.key.lead}</p>

      {failed !== null && <div className="key-said" data-tone="bad" role="alert">
        <TriangleAlert size={15} aria-hidden="true" />
        <p>{sentenceOf(failed) || words.hint.requestFailed}</p>
      </div>}
      {failed === null && form === null && <p className="working" role="status">
        <span className="moon" aria-hidden="true" />{words.hint.key.reading}
      </p>}

      {answer?.kind === 'stored' && <div className="key-done" role="status" data-test="key-done">
        <div className="key-said" data-tone="ok">
          <Check size={15} aria-hidden="true" />
          {/* By the name that people know it by, where the list has one. */}
          <p>{words.hint.key.done(listed?.services.find(known => known.name === answer.name)?.title ?? answer.name, answer.model)}</p>
        </div>
        {answer.keyIn === 'keyring' && <p className="caption">{words.hint.key.inKeyring}</p>}
        {answer.keyIn === 'file' && <div className="key-said" data-tone="warn">
          <TriangleAlert size={15} aria-hidden="true" />
          <p>{words.hint.key.inFile}</p>
        </div>}
        <p className="caption">{words.hint.key.restart}</p>
      </div>}

      {form !== null && listed !== null && answer?.kind !== 'stored' && <form className="key-form" onSubmit={event => void submit(event)}>
        <label className="key-row">
          <span className="key-name">{words.label.key.service}</span>
          <select
            ref={first}
            className="field"
            value={form.adding ? ADD : String(listed.services.findIndex(candidate => candidate.name === form.service))}
            disabled={storing}
            onChange={event => {
              const to = event.target.value === ADD ? null : listed.services[Number(event.target.value)]?.name ?? null;
              setAnswer(null);
              setForm(chosen(form, listed, to));
            }}
            data-test="key-service"
          >
            {listed.services.map((candidate, index) => <option key={candidate.name} value={String(index)}>
              {candidate.title}
            </option>)}
            <option value={ADD}>{words.label.key.ownService}</option>
          </select>
        </label>

        {form.adding && <>
          <label className="key-row">
            <span className="key-name">{words.label.key.name}</span>
            <input className="field mono" value={form.name} disabled={storing} spellCheck={false} autoComplete="off"
              onChange={event => change({ name: event.target.value })} aria-describedby="key-own-name" data-test="key-name" />
          </label>
          <p className="caption key-under" id="key-own-name">{words.hint.key.ownName}</p>
          <label className="key-row">
            <span className="key-name">{words.label.key.address}</span>
            <input className="field mono" value={form.address} disabled={storing} spellCheck={false} autoComplete="off"
              onChange={event => change({ address: event.target.value })} aria-describedby="key-own-address" data-test="key-address" />
          </label>
          <p className="caption key-under" id="key-own-address">{words.hint.key.ownAddress}</p>
        </>}
        {service !== null && <p className="caption key-under mono" title={service.base_url}>{service.base_url}</p>}

        <label className="key-row">
          <span className="key-name">{words.label.key.model}</span>
          <input className="field mono" value={form.model} disabled={storing} spellCheck={false} autoComplete="off"
            onChange={event => change({ model: event.target.value })} aria-describedby="key-model" data-test="key-model" />
        </label>
        <p className="caption key-under" id="key-model">{words.hint.key.modelName}</p>

        <label className="key-row">
          <span className="key-name">{words.label.key.contextWindow}</span>
          <span className="key-with-unit">
            <input className="field mono" value={form.window} disabled={storing} inputMode="numeric" spellCheck={false}
              autoComplete="off" aria-invalid={badWindow || undefined}
              onChange={event => change({ window: event.target.value })} aria-describedby="key-window" data-test="key-window" />
            <span className="caption">{words.label.key.tokens}</span>
          </span>
        </label>
        <p className="caption key-under" id="key-window">{words.hint.key.windowSize}</p>

        {takesKey(form, listed) && <label className="key-row">
          <span className="key-name">{words.label.key.apiKey}</span>
          {/* Never shown, and never offered to be remembered by anything but the program that it is for. */}
          <input ref={keyField} className="field mono" type="password" value={form.key} disabled={storing}
            spellCheck={false} autoComplete="off"
            onChange={event => change({ key: event.target.value })} aria-describedby="key-note" data-test="key-key" />
        </label>}
        {keyNote && <p className="caption key-under" id="key-note" data-test="key-note">{keyNote}</p>}
        {takesKey(form, listed) && <p className="caption key-under">{words.hint.key.whereItGoes}</p>}

        {answer?.kind === 'refused' && <div className="key-said" data-tone="bad" role="alert" data-test="key-refused">
          <TriangleAlert size={15} aria-hidden="true" />
          <p>{sentenceOf(answer.said) || words.hint.requestFailed}</p>
        </div>}

        <div className="key-foot">
          <button type="button" className="btn" disabled={storing} onClick={close}>{words.label.cancel}</button>
          <button type="submit" className="btn btn-main" disabled={storing || !complete(form)} data-test="key-store">
            {storing ? words.label.key.storing : words.label.key.store}
          </button>
        </div>
      </form>}

      {(answer?.kind === 'stored' || failed !== null) && <div className="key-foot">
        <button type="button" ref={done} className="btn btn-main" onClick={close} data-test="key-close">
          {answer?.kind === 'stored' ? words.label.done : words.label.close}
        </button>
      </div>}
    </main>
  </Words.Provider>;
}
