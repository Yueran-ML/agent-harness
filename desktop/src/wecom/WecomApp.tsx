import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import { Check, ChevronRight, ExternalLink, MessageCircle, TriangleAlert } from 'lucide-react';
import { Words } from '../app/locale';
import { messagesFor } from '../i18n';
import { blankInside, hostOf, sentenceOf } from '../key/form';
import { themes } from '../themes';
import { botIdFits, complete, CONSOLE, documentOf, formOf, storedOf, type Form, type Stored } from './form';

/** The words of the window, in the language that it speaks. */
type Messages = ReturnType<typeof messagesFor>;

/** The step after which the bot is stored: those after it are done once it is. */
const STORED_AT = 3;

/** What is done in the admin console, step by step, with the way to the console: all of it, or what is left from a step on. */
function StepList({ words, from = 1 }: { words: Messages; from?: number }) {
  return <>
    <div className="key-link setup-console">
      <button type="button" className="btn btn-quiet" onClick={() => void window.columbinaWecom?.openPage(CONSOLE)}
        data-test="wecom-console">
        <ExternalLink size={14} aria-hidden="true" />{words.label.wecom.console}
      </button>
      <span className="caption mono" title={CONSOLE}>{hostOf(CONSOLE)}</span>
    </div>
    <ol start={from}>
      {words.hint.wecom.steps.map((step, index) => (index + 1 < from ? null : <li key={step}><span>{step}</span></li>))}
    </ol>
  </>;
}

/**
 * The window in which the bot of WeCom and its Secret are given.
 *
 * It shows nothing that a model wrote, and, beside the windows for the key of a service and for
 * the app of Feishu, it is the only place of the application that ever has a secret. What is
 * typed goes to the shell, which hands it to the program that checks it with WeCom and stores
 * it. The answer holds no secret.
 */
export function WecomApp() {
  const [language, setLanguage] = useState('en');
  const words = useMemo(() => messagesFor(language), [language]);
  const [form, setForm] = useState<Form | null>(null);
  // Whether a bot was set up when the window opened: its Secret is stored and not shown.
  const [setUp, setSetUp] = useState(false);
  // What went wrong before there was a form: a sentence, or an empty text for nothing known.
  const [failed, setFailed] = useState<string | null>(null);
  const [storing, setStoring] = useState(false);
  const [answer, setAnswer] = useState<Stored | null>(null);
  const botField = useRef<HTMLInputElement>(null);
  const secretField = useRef<HTMLInputElement>(null);
  const done = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let here = true;
    void (async () => {
      try {
        const context = await window.columbinaWecom!.context();
        if (!here) return;
        setLanguage(context.language);
        document.documentElement.lang = context.language;
        if (themes.some(theme => theme === context.theme)) document.documentElement.dataset.theme = context.theme;
        // The program says why it could not answer in a document of its own. The status of a
        // bot says what went wrong with its assistant under the same name, which is no failure here.
        const status = context.status as { set_up?: unknown; error?: unknown } | null;
        if (status === null || typeof status !== 'object' || !('set_up' in status)) {
          setFailed(typeof status?.error === 'string' ? status.error : '');
          return;
        }
        const read = formOf(context.status);
        setSetUp(read.setUp);
        setForm(read.form);
      } catch (problem) {
        if (here) setFailed(problem instanceof Error ? problem.message : String(problem));
      }
    })();
    return () => { here = false; };
  }, []);

  // The keyboard begins where there is something to type: the Secret of a bot that is set up,
  // which is not shown, and the Bot ID otherwise.
  const begun = useRef(false);
  useEffect(() => {
    if (begun.current || form === null) return;
    begun.current = true;
    (setUp ? secretField : botField).current?.focus();
  }, [form, setUp]);
  // What was stored is said at the top of the window, where it is read first, and the keyboard
  // goes to the button that closes the window without taking the view away from it.
  useEffect(() => {
    if (answer?.kind !== 'stored') return;
    document.body.scrollTop = 0;
    document.documentElement.scrollTop = 0;
    done.current?.focus({ preventScroll: true });
  }, [answer]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !event.isComposing && !storing) {
        event.preventDefault();
        void window.columbinaWecom?.close();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [storing]);

  const close = () => void window.columbinaWecom?.close();
  const change = (given: Partial<Form>) => {
    setAnswer(null);
    setForm(current => (current === null ? current : { ...current, ...given }));
  };
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (form === null || storing || !complete(form)) return;
    setStoring(true);
    setAnswer(null);
    try {
      const told = storedOf(await window.columbinaWecom!.store(documentOf(form)));
      setAnswer(told);
      // Once it is stored, the Secret has no business in this window any more.
      if (told.kind === 'stored') setForm(current => (current === null ? current : { ...current, secret: '' }));
    } catch (problem) {
      setAnswer({ kind: 'refused', said: problem instanceof Error ? problem.message : String(problem) });
    } finally {
      setStoring(false);
    }
  };

  const labels = words.label.wecom;
  const hints = words.hint.wecom;
  // A Bot ID that is not written yet is missing, and nothing is wrong with it.
  const badId = form !== null && form.botId.trim() !== '' && !botIdFits(form.botId);
  const blank = form !== null && blankInside(form.secret);

  return <Words.Provider value={words}>
    <main className="key-page setup-page" data-test="wecom-page">
      <header className="key-head">
        <MessageCircle size={18} aria-hidden="true" />
        <h1 className="display">{labels.title}</h1>
      </header>
      <p className="key-lead">{hints.windowLead}</p>

      {failed !== null && <div className="key-said" data-tone="bad" role="alert">
        <TriangleAlert size={15} aria-hidden="true" />
        <p>{sentenceOf(failed) || words.hint.requestFailed}</p>
      </div>}
      {failed === null && form === null && <p className="working" role="status">
        <span className="moon" aria-hidden="true" />{words.hint.key.reading}
      </p>}

      {answer?.kind === 'stored' && form !== null && <div className="key-done" role="status" data-test="wecom-done">
        <div className="key-said" data-tone="ok">
          <Check size={15} aria-hidden="true" />
          <p>{hints.done(answer.botId)}</p>
        </div>
        {answer.secretIn === 'keyring' && <p className="caption">{hints.inKeyring}</p>}
        {answer.secretIn === 'file' && <div className="key-said" data-tone="warn">
          <TriangleAlert size={15} aria-hidden="true" />
          <p>{hints.inFile}</p>
        </div>}
        {/* What is left to do once the bot is stored. */}
        <section className="setup-steps" data-test="wecom-steps">
          <h2 className="overline" data-test="wecom-next">{labels.next}</h2>
          <StepList words={words} from={STORED_AT} />
        </section>
      </div>}

      {form !== null && answer?.kind !== 'stored' && <form className="key-form" onSubmit={event => void submit(event)}>
        <label className="key-row">
          <span className="key-name">{labels.botId}</span>
          <input ref={botField} className="field mono" value={form.botId} disabled={storing} spellCheck={false}
            autoComplete="off" aria-invalid={badId || undefined} aria-describedby="wecom-bot-id"
            onChange={event => change({ botId: event.target.value })} data-test="wecom-bot-id" />
        </label>
        <p className="caption key-under" id="wecom-bot-id" data-tone={badId ? 'bad' : undefined}>
          {badId ? hints.botIdWrong : hints.botId}
        </p>

        <label className="key-row">
          <span className="key-name">{labels.secret}</span>
          {/* Never shown, and never offered to be remembered by anything but the program that it is for.
              The field is given no value by the page: what is given one is written into the markup
              of the page as well, where whatever reads the page as text would find the secret. */}
          <input ref={secretField} className="field mono" type="password" disabled={storing}
            spellCheck={false} autoComplete="off" aria-describedby="wecom-secret"
            onChange={event => change({ secret: event.target.value })} data-test="wecom-secret" />
        </label>
        {blank && <p className="caption key-under" data-tone="bad" data-test="wecom-blank">{hints.blankInSecret}</p>}
        {setUp && <p className="caption key-under" data-test="wecom-secret-again">{hints.secretAgain}</p>}
        <p className="caption key-under" id="wecom-secret">{hints.secret}</p>

        {answer?.kind === 'refused' && <div className="key-said" data-tone="bad" role="alert" data-test="wecom-refused">
          <TriangleAlert size={15} aria-hidden="true" />
          <p>{sentenceOf(answer.said) || words.hint.requestFailed}</p>
        </div>}

        {/* Open where no bot is set up: the first steps give what the form asks for. */}
        <details className="setup-steps" open={!setUp} data-test="wecom-steps">
          <summary><ChevronRight size={14} aria-hidden="true" />{labels.steps}</summary>
          <StepList words={words} />
        </details>

        {/* It stays at the foot of the window while the steps above it are read. */}
        <div className="key-foot setup-foot">
          <button type="button" className="btn" disabled={storing} onClick={close}>{words.label.cancel}</button>
          <button type="submit" className="btn btn-main" disabled={storing || !complete(form)} data-test="wecom-store">
            {storing ? labels.storing : labels.store}
          </button>
        </div>
      </form>}

      {(answer?.kind === 'stored' || failed !== null) && <div className="key-foot">
        <button type="button" ref={done} className="btn btn-main" onClick={close} data-test="wecom-close">
          {answer?.kind === 'stored' ? words.label.done : words.label.close}
        </button>
      </div>}
    </main>
  </Words.Provider>;
}
