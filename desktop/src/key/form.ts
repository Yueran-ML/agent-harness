/**
 * What the window for the key works out before it shows or sends anything.
 * Nothing here draws, so all of it can be tried with plain values.
 */

/** One service, as `columbina setup --list` tells of it. */
export interface ListedService {
  name: string;
  title: string;
  base_url: string;
  own: boolean;
  model: string | null;
  context_window: number | null;
  key_env: string | null;
  key: string;
}

/** What `columbina setup --list` answers. */
export interface Listed {
  use: string | null;
  services: ListedService[];
}

/** What is written in the window. */
export interface Form {
  /** Whether a service of the user's own is being added. `name` and `address` are of that one. */
  adding: boolean;
  /** The name of the listed service that the form is about. Empty while one is added. */
  service: string;
  name: string;
  address: string;
  model: string;
  /** How many tokens the model takes, as it was typed. Empty for not said. */
  window: string;
  key: string;
}

const text = (value: unknown): string => (typeof value === 'string' ? value : '');

/** Read what the shell says is set up. Anything that is not a list of services is none. */
export function listedOf(given: unknown): Listed | null {
  if (given === null || typeof given !== 'object') return null;
  const { use, services } = given as { use?: unknown; services?: unknown };
  if (!Array.isArray(services)) return null;
  const read: ListedService[] = [];
  for (const service of services as unknown[]) {
    if (service === null || typeof service !== 'object') continue;
    const said = service as Record<string, unknown>;
    const name = text(said.name);
    if (name === '') continue;
    const window = said.context_window;
    read.push({
      name,
      title: text(said.title) || name,
      base_url: text(said.base_url),
      own: said.own === true,
      model: text(said.model) || null,
      context_window: typeof window === 'number' && Number.isInteger(window) && window > 0 ? window : null,
      key_env: text(said.key_env) || null,
      key: text(said.key) || 'none',
    });
  }
  return { use: typeof use === 'string' ? use : null, services: read };
}

/**
 * The form as the window opens with it: the service that was asked for, or
 * the one in use, or the first of the list, with what is known of its model.
 * Without any service it begins at a service of the user's own.
 */
export function formFor(listed: Listed, asked: string | null): Form {
  const begin = listed.services.find(service => service.name === asked)
    ?? listed.services.find(service => service.name === listed.use)
    ?? listed.services[0];
  const empty: Form = { adding: true, service: '', name: '', address: '', model: '', window: '', key: '' };
  return chosen(empty, listed, begin?.name ?? null);
}

/**
 * The form after another service was chosen, or after the user chose to add
 * one, which is `null`. What is known of the service takes the place of what
 * stood there, and a key that was typed for another service is not kept.
 */
export function chosen(form: Form, listed: Listed, service: string | null): Form {
  const found = listed.services.find(candidate => candidate.name === service);
  if (!found) return { ...form, adding: true, service: '', model: '', window: '', key: '' };
  return {
    ...form,
    adding: false,
    service: found.name,
    model: found.model ?? '',
    window: found.context_window === null ? '' : String(found.context_window),
    key: '',
  };
}

/** The listed service that a form is about, or null while it adds one. */
export function serviceOf(form: Form, listed: Listed): ListedService | null {
  if (form.adding) return null;
  return listed.services.find(service => service.name === form.service) ?? null;
}

/** Whether the service of a form takes a key at all. One that is being added may. */
export function takesKey(form: Form, listed: Listed): boolean {
  const service = serviceOf(form, listed);
  return service === null || service.key_env !== null;
}

/** How many tokens were typed: null where nothing was, and NaN where it is no number of tokens. */
export function tokensOf(typed: string): number | null {
  const written = typed.replace(/[\s,_]/g, '');
  if (written === '') return null;
  return /^[1-9]\d{0,11}$/.test(written) ? Number(written) : Number.NaN;
}

/** Whether enough is written to send it. The program checks the rest and says what is wrong. */
export function complete(form: Form): boolean {
  if (form.model.trim() === '' || Number.isNaN(tokensOf(form.window))) return false;
  return !form.adding || (form.name.trim() !== '' && form.address.trim() !== '');
}

/**
 * What is sent to `columbina setup --from-stdin` for a form.
 *
 * An address goes along only for a service that is being added, and a key
 * only where one was typed and the service takes one.
 */
export function documentOf(form: Form, listed: Listed): Record<string, string | number> {
  const document: Record<string, string | number> = {
    service: form.adding ? form.name.trim() : form.service,
    model: form.model.trim(),
  };
  if (form.adding) document.base_url = form.address.trim();
  const tokens = tokensOf(form.window);
  if (tokens !== null && !Number.isNaN(tokens)) document.context_window = tokens;
  if (form.key.trim() !== '' && takesKey(form, listed)) document.api_key = form.key.trim();
  return document;
}

/**
 * A sentence that the program wrote, as it is shown: with a capital at its
 * beginning and a full stop at its end. The program begins without the one
 * and ends without the other.
 */
export function sentenceOf(said: string): string {
  const trimmed = said.trim();
  if (trimmed === '') return '';
  const begun = trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
  return /[.!?。！？]$/.test(begun) ? begun : `${begun}.`;
}

/** What the program answered to what was sent: what it stored, or what it says is wrong. */
export type Stored =
  | { kind: 'stored'; name: string; model: string; keyIn: 'keyring' | 'file' | null }
  | { kind: 'refused'; said: string };

export function storedOf(answer: unknown): Stored {
  const given = (answer !== null && typeof answer === 'object' ? answer : {}) as {
    stored?: unknown; key_in?: unknown; error?: unknown;
  };
  const stored = given.stored !== null && typeof given.stored === 'object'
    ? given.stored as { name?: unknown; model?: unknown } : null;
  if (stored !== null && typeof stored.name === 'string' && typeof stored.model === 'string') {
    const keyIn = given.key_in === 'keyring' || given.key_in === 'file' ? given.key_in : null;
    return { kind: 'stored', name: stored.name, model: stored.model, keyIn };
  }
  return { kind: 'refused', said: typeof given.error === 'string' ? given.error : '' };
}
