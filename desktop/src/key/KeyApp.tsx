import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import { Check, ExternalLink, KeyRound, TriangleAlert } from 'lucide-react';
import { Words } from '../app/locale';
import { messagesFor } from '../i18n';
import { themes } from '../themes';
import {
  blankInside, called, checkable, checkedOf, checkOf, chosen, complete, documentOf, forgottenOf, formFor, hostOf, keyKept, listedOf,
  looksLikeAKey, nameFits, onlyTheKey, sentenceOf, serviceCalled, serviceOf, storedOf, takesKey, tokensOf,
  type Checked, type Forgotten, type Form, type Listed, type ListedService, type Stored,
} from './form';

/** The words of the window, in the language that it speaks. */
type Messages = ReturnType<typeof messagesFor>;

/** What is said of a check: the models that the service lists, or why it could not be asked. */
function sayChecked(words: Messages, checked: Checked, service: ListedService | null, name: string): string {
  if (checked.kind === 'models') {
    return checked.models.length > 0 ? words.hint.key.reached(name, checked.models.length) : words.hint.key.noModels(name);
  }
  // A service on this computer that does not answer has not been started.
  if (service !== null && service.key_env === null && checked.failure === 'network') {
    const known = service.name === 'ollama' || service.name === 'lmstudio' ? service.name : 'other';
    return words.hint.key.notRunning[known];
  }
  const said = words.hint.key.failed[checked.failure];
  return checked.failure === 'unknown' && checked.said ? `${said} ${sentenceOf(checked.said)}` : said;
}

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
  // Whether what was stored last was a key and nothing else. A key needs no new start.
  const [keyAlone, setKeyAlone] = useState(false);
  // The service that the window was opened for: the one in use, where the main window knows it.
  const [asked, setAsked] = useState<string | null>(null);
  // What the service said when it was asked for its models with what the form holds.
  const [checking, setChecking] = useState(false);
  const [checked, setChecked] = useState<Checked | null>(null);
  // Each check is counted, so that one that was overtaken by another service says nothing.
  const checks = useRef(0);
  // The deletion of a stored key: asked about, under way, and what it did, for which service.
  const [forgetting, setForgetting] = useState<'asking' | 'working' | null>(null);
  const [forgot, setForgot] = useState<{ name: string; told: Forgotten } | null>(null);
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
        setAsked(typeof context.service === 'string' ? context.service : null);
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
  // The field of the key holds what was typed in it and is not told what to hold. So when the
  // key is taken out of the form, as it is when another service is chosen, the field is emptied.
  const typedKey = form?.key ?? '';
  useEffect(() => {
    if (typedKey === '' && keyField.current) keyField.current.value = '';
  }, [typedKey]);

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
  /** Have what was checked, and a check that is under way, say nothing any more. */
  const unchecked = () => {
    checks.current += 1;
    setChecking(false);
    setChecked(null);
  };
  const change = (given: Partial<Form>) => {
    setAnswer(null);
    setForgot(null);
    // A check was of a service, an address and a key. A model or a size that is written changes none of them.
    if ('key' in given || 'address' in given || 'name' in given) unchecked();
    setForm(current => (current === null ? current : { ...current, ...given }));
  };
  const check = useCallback(async (asking: Form, list: Listed) => {
    checks.current += 1;
    const mine = checks.current;
    setChecking(true);
    setChecked(null);
    let told: Checked;
    try {
      told = checkedOf(await window.columbinaKey!.check(checkOf(asking, list)));
    } catch (problem) {
      told = { kind: 'failed', failure: 'unknown', said: problem instanceof Error ? problem.message : String(problem) };
    }
    if (checks.current !== mine) return;
    setChecked(told);
    setChecking(false);
  }, []);
  // A service on this computer is asked as soon as it is chosen: whether it runs is what the user wants to know.
  const latest = useRef({ form, listed });
  latest.current = { form, listed };
  const chosenHere = form !== null && listed !== null ? serviceOf(form, listed) : null;
  const local = chosenHere !== null && chosenHere.key_env === null ? chosenHere.name : null;
  useEffect(() => {
    const { form: now, listed: list } = latest.current;
    if (local !== null && now !== null && list !== null) void check(now, list);
  }, [local, check]);
  const forget = async (service: ListedService) => {
    setForgetting('working');
    let told: Forgotten;
    try {
      told = forgottenOf(await window.columbinaKey!.forget(service.name));
      if (told.kind === 'forgotten') {
        // The list says anew where the key of each service comes from.
        const read = listedOf((await window.columbinaKey!.context()).list);
        if (read !== null) setListed(read);
      }
    } catch (problem) {
      told = { kind: 'refused', said: problem instanceof Error ? problem.message : String(problem) };
    }
    setForgot({ name: service.name, told });
    setForgetting(null);
  };
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (form === null || listed === null || storing || !complete(form)) return;
    setStoring(true);
    setAnswer(null);
    try {
      const told = storedOf(await window.columbinaKey!.store(documentOf(form, listed)));
      setKeyAlone(onlyTheKey(form, listed, asked));
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
  // A name that is not written yet is missing, and nothing is wrong with it.
  const badName = form !== null && form.adding && form.name.trim() !== '' && !nameFits(form.name);
  // The models that the service listed when it was checked, which the field of the model offers.
  const models = checked?.kind === 'models' ? checked.models : [];
  const model = form?.model.trim() ?? '';
  const named = service !== null ? called(service) : form?.name.trim() || words.label.key.ownService;
  const blank = form !== null && blankInside(form.key);

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
          <p>{words.hint.key.done(serviceCalled(answer.name, listed), answer.model)}</p>
        </div>
        {answer.keyIn === 'keyring' && <p className="caption">{words.hint.key.inKeyring}</p>}
        {answer.keyIn === 'file' && <div className="key-said" data-tone="warn">
          <TriangleAlert size={15} aria-hidden="true" />
          <p>{words.hint.key.inFile}</p>
        </div>}
        <p className="caption" data-test="key-when">{keyAlone ? words.hint.key.atOnce : words.hint.key.restart}</p>
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
              setForgot(null);
              setForgetting(null);
              unchecked();
              setForm(chosen(form, listed, to));
            }}
            data-test="key-service"
          >
            {listed.services.map((candidate, index) => <option key={candidate.name} value={String(index)}>
              {called(candidate)}
            </option>)}
            <option value={ADD}>{words.label.key.ownService}</option>
          </select>
        </label>

        {form.adding && <>
          <label className="key-row">
            <span className="key-name">{words.label.key.name}</span>
            <input className="field mono" value={form.name} disabled={storing} spellCheck={false} autoComplete="off"
              aria-invalid={badName || undefined}
              onChange={event => change({ name: event.target.value })} aria-describedby="key-own-name" data-test="key-name" />
          </label>
          {/* What a name may be stands here from the beginning. It is marked once a name breaks it. */}
          <p className="caption key-under" id="key-own-name" data-tone={badName ? 'bad' : undefined}>{words.hint.key.ownName}</p>
          <label className="key-row">
            <span className="key-name">{words.label.key.address}</span>
            <input className="field mono" value={form.address} disabled={storing} spellCheck={false} autoComplete="off"
              onChange={event => change({ address: event.target.value })} aria-describedby="key-own-address" data-test="key-address" />
          </label>
          <p className="caption key-under" id="key-own-address">{words.hint.key.ownAddress}</p>
        </>}
        {service !== null && <p className="caption key-under mono" title={service.base_url}>{service.base_url}</p>}
        {service?.key_page && <div className="key-under key-link">
          {/* Where the browser goes stands beside the button: the machine, and the whole address when it is pointed at. */}
          <button type="button" className="btn btn-quiet" onClick={() => void window.columbinaKey?.openPage(service.key_page!)}
            data-test="key-get">
            <ExternalLink size={14} aria-hidden="true" />{words.label.key.getKey}
          </button>
          <span className="caption mono" title={service.key_page}>{hostOf(service.key_page)}</span>
        </div>}

        <label className="key-row">
          <span className="key-name">{words.label.key.model}</span>
          <input className="field mono" value={form.model} disabled={storing} spellCheck={false} autoComplete="off"
            list={models.length > 0 ? 'key-models' : undefined}
            onChange={event => change({ model: event.target.value })} aria-describedby="key-model" data-test="key-model" />
        </label>
        {models.length > 0 && <datalist id="key-models" data-test="key-models">
          {models.map(each => <option key={each} value={each} />)}
        </datalist>}
        {looksLikeAKey(form.model)
          ? <p className="caption key-under" data-tone="warn" data-test="key-like-a-key">{words.hint.key.likeAKey}</p>
          : models.length > 0 && model !== '' && !models.includes(model)
            ? <p className="caption key-under" data-tone="warn" data-test="key-not-listed">{words.hint.key.notListed(model)}</p>
            : <p className="caption key-under" id="key-model">{words.hint.key.modelName}</p>}

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
          {/* Never shown, and never offered to be remembered by anything but the program that it is for.
              The field is given no value by the page: what is given one is written into the markup
              of the page as well, where whatever reads the page as text would find the key. */}
          <input ref={keyField} className="field mono" type="password" disabled={storing}
            spellCheck={false} autoComplete="off"
            onChange={event => change({ key: event.target.value })} aria-describedby="key-note" data-test="key-key" />
        </label>}
        {blank && <p className="caption key-under" data-tone="bad" data-test="key-blank">{words.hint.key.blankInKey}</p>}
        {keyNote && <p className="caption key-under" id="key-note" data-test="key-note">{keyNote}</p>}
        {takesKey(form, listed) && <p className="caption key-under">{words.hint.key.whereItGoes}</p>}
        {service !== null && keyKept(service) && forgetting !== 'asking' && <div className="key-under">
          <button type="button" className="btn btn-quiet" disabled={storing || forgetting === 'working'}
            onClick={() => setForgetting('asking')} data-test="key-forget">
            {words.label.key.forget}
          </button>
        </div>}
        {service !== null && forgetting === 'asking' && <div className="key-said" data-tone="warn" role="alert" data-test="key-forget-ask">
          <TriangleAlert size={15} aria-hidden="true" />
          <div className="key-ask">
            <p>{words.hint.key.forgetAsk(called(service))}</p>
            <div className="key-ask-actions">
              <button type="button" className="btn" onClick={() => setForgetting(null)} data-test="key-forget-keep">
                {words.label.key.keep}
              </button>
              <button type="button" className="btn btn-bad" onClick={() => void forget(service)} data-test="key-forget-sure">
                {words.label.key.forgetConfirm}
              </button>
            </div>
          </div>
        </div>}
        {forgot !== null && <div className="key-said" data-tone={forgot.told.kind === 'forgotten' ? 'ok' : 'bad'} role="status"
          data-test="key-forgot">
          {forgot.told.kind === 'forgotten' ? <Check size={15} aria-hidden="true" /> : <TriangleAlert size={15} aria-hidden="true" />}
          <p>{forgot.told.kind === 'refused' ? sentenceOf(forgot.told.said) || words.hint.requestFailed
            : forgot.told.key ? words.hint.key.forgotten(serviceCalled(forgot.name, listed))
              : words.hint.key.noneToForget(serviceCalled(forgot.name, listed))}</p>
        </div>}

        {checking && <p className="working" role="status" data-test="key-checking">
          <span className="moon" aria-hidden="true" />{words.label.key.checking}
        </p>}
        {checked !== null && <div className="key-said" data-tone={checked.kind === 'models' ? 'ok' : 'bad'} role="status"
          data-test="key-checked">
          {checked.kind === 'models' ? <Check size={15} aria-hidden="true" /> : <TriangleAlert size={15} aria-hidden="true" />}
          <p>{sayChecked(words, checked, service, named)}</p>
        </div>}

        {answer?.kind === 'refused' && <div className="key-said" data-tone="bad" role="alert" data-test="key-refused">
          <TriangleAlert size={15} aria-hidden="true" />
          <p>{sentenceOf(answer.said) || words.hint.requestFailed}</p>
        </div>}

        <div className="key-foot">
          <button type="button" className="btn" disabled={storing} onClick={close}>{words.label.cancel}</button>
          <button type="button" className="btn" disabled={storing || checking || !checkable(form)}
            onClick={() => void check(form, listed)} data-test="key-check">
            {checking ? words.label.key.checking : words.label.key.check}
          </button>
          <button type="submit" className="btn btn-main" disabled={storing || !complete(form) || blank} data-test="key-store">
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
