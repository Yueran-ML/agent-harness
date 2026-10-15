// @vitest-environment jsdom
/**
 * The window for the key, mounted over a shell that the test plays. Every key that is typed
 * here is made up.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { KeyApp } from '../src/key/KeyApp';
import { likeABrowser, service } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  delete document.documentElement.dataset.theme;
  document.documentElement.lang = '';
});

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" in the window.`);
  return found;
};
const field = (name: string) => one(name) as HTMLInputElement;
const write = (name: string, text: string) => { fireEvent.change(one(name), { target: { value: text } }); };
/** Let everything that is on its way arrive. */
const settled = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
/** A promise that the test keeps in its hand. */
function held<T>() {
  let release!: (value: T) => void;
  const promise = new Promise<T>((resolve) => { release = resolve; });
  return { promise, release };
}

/** Three services: one in use without a key, one with a stored key, one of the user's own that asks for none. */
const listed = () => ({
  use: 'deepseek',
  services: [
    { ...service('deepseek', 'DeepSeek', { key_env: 'DEEPSEEK_API_KEY' }), model: 'deepseek-flash', context_window: 1_048_576 },
    { ...service('openai', 'OpenAI', { key: 'stored', key_env: 'OPENAI_API_KEY' }), model: 'gpt-test', context_window: 128_000 },
    { ...service('lab', 'http://10.0.0.5:8000/v1', { own: true, key_env: null }), model: 'lab-model', context_window: null },
  ],
});
const KEY = 'sk-made-up-for-this-test';

/** Open the window over a shell that answers as the test says. */
async function opened(given: { service?: string | null; language?: string; theme?: string; list?: unknown } = {}) {
  const shell = {
    context: vi.fn(async () => ({ service: 'deepseek' as string | null, language: 'en', theme: 'moonveil', list: listed() as unknown, ...given })),
    store: vi.fn(async (_document: Record<string, string | number>): Promise<unknown> => ({ stored: { name: 'deepseek', model: 'deepseek-flash' }, key_in: 'keyring' })),
    check: vi.fn(async (_document: Record<string, string>): Promise<unknown> => ({ models: ['deepseek-flash', 'deepseek-pro'] })),
    forget: vi.fn(async (name: string): Promise<unknown> => ({ forgotten: { name, key: true, chosen: false } })),
    openPage: vi.fn(async (_address: string) => {}),
    close: vi.fn(async () => {}),
  };
  vi.stubGlobal('columbinaKey', shell);
  render(<KeyApp />);
  await settled();
  return shell;
}
const store = async () => { await act(async () => { fireEvent.submit(one('key-store').closest('form')!); }); };
const options = () => [...(one('key-service') as HTMLSelectElement).options].map(option => option.textContent);
const choose = (says: string) => {
  const select = one('key-service') as HTMLSelectElement;
  const option = [...select.options].find(each => each.textContent === says);
  if (!option) throw new Error(`No service is called "${says}".`);
  fireEvent.change(select, { target: { value: option.value } });
};

describe('the window for the key', () => {
  it('opens at the service that it was asked for, with what is known of it, and begins at the key', async () => {
    const shell = await opened();
    expect(shell.context).toHaveBeenCalledTimes(1);
    expect(one('key-page').querySelector('h1')?.textContent).toBe('Model service');
    expect(options()).toEqual(['DeepSeek', 'OpenAI', 'lab', 'A service of your own']);
    expect((one('key-service') as HTMLSelectElement).selectedOptions[0].textContent).toBe('DeepSeek');
    expect([field('key-model').value, field('key-window').value, field('key-key').value]).toEqual(['deepseek-flash', '1048576', '']);
    expect(one('key-note').textContent).toBe('No key is there for this service yet.');
    // The service is set up and lacks a key: the key is what is typed next.
    expect(document.activeElement).toBe(one('key-key'));
    expect(marked('key-name')).toEqual([]);
    expect((one('key-store') as HTMLButtonElement).disabled).toBe(false);
    expect(one('key-page').textContent).toContain('The key goes from this window to Columbina');
    // The keyboard stays where the user put it, whatever is written.
    one('key-model').focus();
    write('key-model', 'deepseek-other');
    expect(document.activeElement).toBe(one('key-model'));
  });

  it('opens at the service in use where none was asked for, and at the first where none is in use', async () => {
    await opened({ service: null });
    expect((one('key-service') as HTMLSelectElement).selectedOptions[0].textContent).toBe('DeepSeek');
    cleanup();
    await opened({ service: null, list: { ...listed(), use: null } });
    expect((one('key-service') as HTMLSelectElement).selectedOptions[0].textContent).toBe('DeepSeek');
    cleanup();
    await opened({ service: 'openai' });
    expect((one('key-service') as HTMLSelectElement).selectedOptions[0].textContent).toBe('OpenAI');
  });

  it('never shows the key, and offers it to nothing that would remember it', async () => {
    await opened();
    const key = field('key-key');
    expect([key.type, key.autocomplete, key.getAttribute('spellcheck')]).toEqual(['password', 'off', 'false']);
    write('key-key', KEY);
    expect(key.value).toBe(KEY);
    expect(document.body.textContent).not.toContain(KEY);
    expect(document.body.innerHTML).not.toContain(KEY);
  });

  it('hands what is written to the shell, with a key only where one was typed', async () => {
    const shell = await opened();
    await store();
    expect(shell.store.mock.calls).toEqual([[{ service: 'deepseek', model: 'deepseek-flash', context_window: 1_048_576 }]]);
  });

  it('says what was stored, has the key no longer, and says that a key alone needs no new start', async () => {
    const shell = await opened();
    write('key-key', `  ${KEY} `);
    await store();
    expect(shell.store.mock.calls).toEqual([[{ service: 'deepseek', model: 'deepseek-flash', context_window: 1_048_576, api_key: KEY }]]);
    const done = one('key-done');
    expect([...done.querySelectorAll('p')].map(said => said.textContent)).toEqual([
      'Columbina talks to DeepSeek now, with the model deepseek-flash.',
      'The key is in the keyring of the system.',
      'Columbina looks for the key with every request, so it needs no new start.',
    ]);
    // The form is gone, and with it the field that held the key.
    expect(marked('key-key')).toEqual([]);
    expect(document.body.innerHTML).not.toContain(KEY);
    expect(document.activeElement).toBe(one('key-close'));
    expect(one('key-close').textContent).toBe('Done');
    fireEvent.click(one('key-close'));
    expect(shell.close).toHaveBeenCalledTimes(1);
  });

  it('needs no new start for a key of the service that the program talks to by its profile', async () => {
    // No service was ever chosen. The window was opened for the one that the program talks to.
    const shell = await opened({ service: 'deepseek', list: { ...listed(), use: null } });
    write('key-key', KEY);
    await store();
    expect(shell.store).toHaveBeenCalledTimes(1);
    expect(one('key-when').textContent).toBe('Columbina looks for the key with every request, so it needs no new start.');
  });

  it('says that a new start is needed where more than a key was stored', async () => {
    const shell = await opened();
    shell.store.mockResolvedValueOnce({ stored: { name: 'deepseek', model: 'deepseek-other' }, key_in: null });
    write('key-model', 'deepseek-other');
    write('key-key', KEY);
    await store();
    expect([...one('key-done').querySelectorAll('p')].map(said => said.textContent)).toEqual([
      'Columbina talks to DeepSeek now, with the model deepseek-other.',
      'What was stored is used once Columbina is started again.',
    ]);
  });

  it('says so where no keyring took the key', async () => {
    const shell = await opened();
    shell.store.mockResolvedValueOnce({ stored: { name: 'deepseek', model: 'deepseek-flash' }, key_in: 'file' });
    write('key-key', KEY);
    await store();
    const warned = one('key-done').querySelector('[data-tone="warn"]');
    expect(warned?.textContent).toBe('No keyring took the key. It lies in a file in the folder of Columbina, as it is and not encrypted.');
    expect(one('key-done').textContent).not.toContain('The key is in the keyring of the system.');
  });

  it('says what the program refused, in a sentence, until something is changed', async () => {
    const shell = await opened();
    shell.store.mockResolvedValueOnce({ error: 'the service knows no such model' });
    write('key-key', KEY);
    await store();
    expect(one('key-refused').getAttribute('role')).toBe('alert');
    expect(one('key-refused').textContent).toBe('The service knows no such model.');
    // The form is still there to be put right, with what was typed.
    expect(field('key-key').value).toBe(KEY);
    expect(marked('key-done')).toEqual([]);
    write('key-model', 'deepseek-other');
    expect(marked('key-refused')).toEqual([]);
    // Nor is it said of another service that is chosen after it.
    shell.store.mockResolvedValueOnce({ error: 'the service knows no such model' });
    await store();
    expect(marked('key-refused')).toHaveLength(1);
    choose('OpenAI');
    expect(marked('key-refused')).toEqual([]);
  });

  it('says that it did not go through where the shell fails, and where the program says nothing', async () => {
    const shell = await opened();
    shell.store.mockRejectedValueOnce(new Error('the program of the service did not start'));
    await store();
    expect(one('key-refused').textContent).toBe('The program of the service did not start.');
    shell.store.mockResolvedValueOnce({});
    await store();
    expect(one('key-refused').textContent).toBe('That did not go through.');
    expect((one('key-store') as HTMLButtonElement).disabled).toBe(false);
  });

  it('stores once at a time, and is not closed by Esc while it stores', async () => {
    const shell = await opened();
    const storing = held<unknown>();
    shell.store.mockResolvedValueOnce({ error: 'the keyring is locked' }).mockReturnValueOnce(storing.promise);
    write('key-key', KEY);
    await store();
    expect(marked('key-refused')).toHaveLength(1);
    await store();
    await store();
    expect(shell.store).toHaveBeenCalledTimes(2);
    // What was refused before is not said of what is being stored now.
    expect(marked('key-refused')).toEqual([]);
    expect(one('key-store').textContent).toBe('Storing');
    expect(['key-service', 'key-model', 'key-window', 'key-key', 'key-store'].map(name => (one(name) as HTMLInputElement).disabled))
      .toEqual([true, true, true, true, true]);
    expect(!fireEvent.keyDown(document.body, { key: 'Escape' })).toBe(false);
    // Nor is it left by its button: what is being stored would be stored behind a closed window.
    const cancel = [...document.querySelectorAll('button')].find(each => each.textContent === 'Cancel')!;
    expect(cancel.disabled).toBe(true);
    fireEvent.click(cancel);
    expect(shell.close).not.toHaveBeenCalled();
    await act(async () => { storing.release({ stored: { name: 'deepseek', model: 'deepseek-flash' }, key_in: 'keyring' }); });
    expect(marked('key-done')).toHaveLength(1);
  });

  it('is closed by Esc and by its button, and not by the Esc that ends a composition', async () => {
    const shell = await opened();
    expect(!fireEvent.keyDown(document.body, { key: 'Escape', isComposing: true })).toBe(false);
    expect(!fireEvent.keyDown(document.body, { key: 'Enter' })).toBe(false);
    expect(shell.close).not.toHaveBeenCalled();
    expect(!fireEvent.keyDown(document.body, { key: 'Escape' })).toBe(true);
    expect(shell.close).toHaveBeenCalledTimes(1);
    const cancel = [...document.querySelectorAll('button')].find(each => each.textContent === 'Cancel')!;
    fireEvent.click(cancel);
    expect(shell.close).toHaveBeenCalledTimes(2);
  });

  it('shows what is known of another service that is chosen, and keeps no key that was typed for the first', async () => {
    const shell = await opened();
    write('key-key', KEY);
    choose('OpenAI');
    expect([field('key-model').value, field('key-window').value, field('key-key').value]).toEqual(['gpt-test', '128000', '']);
    expect(one('key-note').textContent).toBe('A key is stored for this service. Leave the field empty to keep it.');
    shell.store.mockResolvedValueOnce({ stored: { name: 'openai', model: 'gpt-test' }, key_in: null });
    await store();
    expect(shell.store.mock.calls).toEqual([[{ service: 'openai', model: 'gpt-test', context_window: 128_000 }]]);
    // Another service than the one in use was chosen: that is more than a key.
    expect(one('key-when').textContent).toBe('What was stored is used once Columbina is started again.');
    expect(one('key-done').querySelector('p')?.textContent).toBe('Columbina talks to OpenAI now, with the model gpt-test.');
  });

  it('has no field for a key where the service asks for none', async () => {
    const shell = await opened();
    choose('lab');
    expect(marked('key-key')).toEqual([]);
    expect(one('key-note').textContent).toBe('This service asks for no key.');
    expect(one('key-page').textContent).not.toContain('The key goes from this window to Columbina');
    expect(field('key-window').value).toBe('');
    // The address of the service stands under its name.
    expect(document.querySelector('.key-under.mono')?.textContent).toBe('http://10.0.0.5:8000/v1');
    shell.store.mockResolvedValueOnce({ stored: { name: 'lab', model: 'lab-model' }, key_in: null });
    await store();
    expect(shell.store.mock.calls).toEqual([[{ service: 'lab', model: 'lab-model' }]]);
    // A service of the user's own is called by the name that the user gave it.
    expect(one('key-done').querySelector('p')?.textContent).toBe('Columbina talks to lab now, with the model lab-model.');
  });

  it('says where the key of a service comes from that has one in a variable or for another address', async () => {
    const from = async (key: string) => {
      cleanup();
      const list = listed();
      list.services[0].key = key;
      await opened({ list });
      return one('key-note').textContent;
    };
    expect(await from('environment'))
      .toBe('The key of this service comes from the variable DEEPSEEK_API_KEY, which goes before a stored key.');
    expect(await from('stored-for-another-address'))
      .toBe('A key is stored for this service, for another address than it has now. It is not sent to this address.');
  });

  it('adds a service of the user\'s own once its name, its address and its model are written', async () => {
    const shell = await opened();
    choose('A service of your own');
    expect([field('key-name').value, field('key-address').value, field('key-model').value, field('key-window').value]).toEqual(['', '', '', '']);
    const able = () => !(one('key-store') as HTMLButtonElement).disabled;
    expect(able()).toBe(false);
    write('key-name', ' my-lab_2 ');
    write('key-address', ' http://10.0.0.9/v1 ');
    expect(able()).toBe(false);
    write('key-model', ' a-model ');
    expect(able()).toBe(true);
    // A service that is being added may take a key.
    write('key-key', KEY);
    write('key-window', '32,000');
    shell.store.mockResolvedValueOnce({ stored: { name: 'my-lab_2', model: 'a-model' }, key_in: 'keyring' });
    await store();
    expect(shell.store.mock.calls).toEqual([[
      { service: 'my-lab_2', base_url: 'http://10.0.0.9/v1', model: 'a-model', context_window: 32_000, api_key: KEY },
    ]]);
    expect(one('key-when').textContent).toBe('What was stored is used once Columbina is started again.');
  });

  it('says under the name what a name of one\'s own may be, and marks one that cannot be a name', async () => {
    const shell = await opened();
    choose('A service of your own');
    write('key-address', 'http://10.0.0.9/v1');
    write('key-model', 'a-model');
    const hint = () => document.getElementById(field('key-name').getAttribute('aria-describedby')!)!;
    const shown = () => [field('key-name').getAttribute('aria-invalid'), hint().dataset.tone, (one('key-store') as HTMLButtonElement).disabled];
    expect(hint().textContent)
      .toBe('A name of your own. It begins with a small letter and goes on with small letters, digits, - and _, 32 characters at most.');
    // Nothing is wrong with a name that is not written yet. It is only missing.
    expect(shown()).toEqual([null, undefined, true]);
    for (const name of ['My-Lab', '2nd', 'my lab', 'my.lab', 'a'.repeat(33), '-lab']) {
      write('key-name', name);
      expect([name, ...shown()]).toEqual([name, 'true', 'bad', true]);
    }
    for (const name of ['a', 'my-lab_2', 'a'.repeat(32), '  lab  ']) {
      write('key-name', name);
      expect([name, ...shown()]).toEqual([name, null, undefined, false]);
    }
    // What cannot be a name is not sent, however the form is sent.
    write('key-name', 'My-Lab');
    await store();
    expect(shell.store).not.toHaveBeenCalled();
  });

  it('marks a number of tokens that is none, and sends none where the field is empty', async () => {
    const shell = await opened();
    const shown = () => [field('key-window').getAttribute('aria-invalid'), (one('key-store') as HTMLButtonElement).disabled];
    write('key-window', 'many');
    expect(shown()).toEqual(['true', true]);
    await store();
    expect(shell.store).not.toHaveBeenCalled();
    write('key-window', '128 000');
    expect(shown()).toEqual([null, false]);
    write('key-window', '');
    expect(shown()).toEqual([null, false]);
    await store();
    expect(shell.store.mock.calls).toEqual([[{ service: 'deepseek', model: 'deepseek-flash' }]]);
  });

  it('says so where what is set up could not be read, and can be closed then', async () => {
    const shell = await opened({ list: { error: 'the file of the services is no YAML' } });
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('The file of the services is no YAML.');
    expect(marked('key-store')).toEqual([]);
    expect(one('key-close').textContent).toBe('Close');
    fireEvent.click(one('key-close'));
    expect(shell.close).toHaveBeenCalledTimes(1);
    cleanup();
    await opened({ list: null });
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('That did not go through.');
  });

  it('says so where the shell does not answer what is set up', async () => {
    const shell = {
      context: vi.fn(async () => { throw new Error('the shell is busy'); }),
      store: vi.fn(async () => ({})),
      close: vi.fn(async () => {}),
    };
    vi.stubGlobal('columbinaKey', shell);
    render(<KeyApp />);
    expect(document.querySelector('[role="status"]')?.textContent).toBe('Reading what is set up.');
    await settled();
    expect(document.querySelector('[role="alert"]')?.textContent).toBe('The shell is busy.');
    expect(document.querySelector('[role="status"]')).toBeNull();
  });

  it('begins at the service where the service that it opens at is not set up yet', async () => {
    const list = listed();
    list.services[0].model = '';
    await opened({ list });
    expect(document.activeElement).toBe(one('key-service'));
    cleanup();
    // Without any service it begins at one of the user's own.
    await opened({ service: null, list: { use: null, services: [] } });
    expect(options()).toEqual(['A service of your own']);
    expect(marked('key-name')).toHaveLength(1);
    expect(document.activeElement).toBe(one('key-service'));
  });

  it('speaks the language and wears the theme of the window that opened it', async () => {
    await opened({ language: 'ja', theme: 'moonwhite' });
    expect([document.documentElement.lang, document.documentElement.dataset.theme]).toEqual(['ja', 'moonwhite']);
    expect(one('key-page').querySelector('h1')?.textContent).not.toBe('Model service');
    cleanup();
    delete document.documentElement.dataset.theme;
    // A theme that this window does not know is not put on.
    await opened({ theme: 'something-else' });
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});

describe('checking a service, and deleting its key', () => {
  const linked = () => {
    const list = listed();
    list.services[0] = { ...list.services[0], key_page: 'https://platform.deepseek.com/api_keys' };
    return list;
  };
  const check = async () => { await act(async () => { fireEvent.click(one('key-check')); }); await settled(); };

  it('offers the page on which a key is made, and names where it leads', async () => {
    const shell = await opened({ list: linked() });
    expect(one('key-get').textContent).toBe('Get a key');
    // The machine stands beside it, and the whole address is shown where it is pointed at.
    const where = one('key-get').nextElementSibling as HTMLElement;
    expect([where.textContent, where.title]).toEqual(['platform.deepseek.com', 'https://platform.deepseek.com/api_keys']);
    fireEvent.click(one('key-get'));
    expect(shell.openPage).toHaveBeenCalledExactlyOnceWith('https://platform.deepseek.com/api_keys');
    // A service that names no such page offers none.
    choose('OpenAI');
    expect(marked('key-get')).toEqual([]);
  });

  it('asks the service for its models with the key that is typed, and offers them for the model', async () => {
    const shell = await opened();
    write('key-key', `  ${KEY}\n`);
    await check();
    expect(shell.check).toHaveBeenCalledExactlyOnceWith({ service: 'deepseek', api_key: KEY });
    expect(one('key-checked').textContent).toBe('DeepSeek answered, and lists 2 models.');
    expect(field('key-model').getAttribute('list')).toBe('key-models');
    expect([...one('key-models').querySelectorAll('option')].map(option => option.value)).toEqual(['deepseek-flash', 'deepseek-pro']);
    // The model that is written is one of them.
    expect(marked('key-not-listed')).toEqual([]);
    // What it found stays while the model is written, and is of no use once the key is another.
    write('key-model', 'deepseek-old');
    expect(one('key-not-listed').textContent).toBe('deepseek-old is not among the models that the service lists.');
    write('key-key', 'sk-another-one');
    expect(marked('key-checked')).toEqual([]);
    expect(field('key-model').getAttribute('list')).toBeNull();
    expect(marked('key-not-listed')).toEqual([]);
  });

  it('says that a service listed no model, and checks one without a typed key with the key that a session sends', async () => {
    const shell = await opened();
    shell.check.mockResolvedValueOnce({ models: [] });
    await check();
    expect(shell.check).toHaveBeenCalledExactlyOnceWith({ service: 'deepseek' });
    expect(one('key-checked').textContent).toBe('DeepSeek answered, and lists no model.');
    expect(marked('key-models')).toEqual([]);
  });

  it('says for each kind of failure what to look at', async () => {
    const shell = await opened();
    const kinds: [string, string][] = [
      ['key', 'The service does not take this key.'],
      ['forbidden', 'The service takes the key and does not allow this'],
      ['network', 'The service could not be reached.'],
      ['timeout', 'The service did not answer in time.'],
      ['address', 'Nothing that serves models answers at this address.'],
      ['busy', 'The service is busy.'],
    ];
    for (const [kind, said] of kinds) {
      shell.check.mockResolvedValueOnce({ error: 'what the program said', kind });
      await check();
      expect(one('key-checked').textContent, kind).toContain(said);
      expect(one('key-checked').dataset.tone).toBe('bad');
    }
    // A failure of no known kind says what the program said.
    shell.check.mockResolvedValueOnce({ error: 'the service answered 418', kind: 'strange' });
    await check();
    expect(one('key-checked').textContent).toBe('The check did not succeed. The service answered 418.');
    shell.check.mockRejectedValueOnce(new Error('The setup program did not end in time.'));
    await check();
    expect(one('key-checked').textContent).toBe('The check did not succeed. The setup program did not end in time.');
  });

  it('asks a service on this computer as soon as it is chosen, and says how to start one that does not run', async () => {
    const list = listed();
    list.services.push({ ...service('ollama', 'Ollama', { key_env: null }), model: 'qwen2.5:7b', context_window: null });
    const shell = await opened({ list });
    expect(shell.check).not.toHaveBeenCalled();
    shell.check.mockResolvedValueOnce({ error: 'the service could not be reached', kind: 'network' });
    choose('Ollama');
    await settled();
    expect(shell.check).toHaveBeenCalledExactlyOnceWith({ service: 'ollama' });
    expect(one('key-checked').textContent).toBe(
      'Ollama does not answer on this computer. Start it, for instance with ollama serve, and check again.',
    );
    shell.check.mockResolvedValueOnce({ error: 'the service could not be reached', kind: 'network' });
    choose('lab');
    await settled();
    expect(one('key-checked').textContent).toBe('The service does not answer on this computer. Start it, and check again.');
    // A service far away that cannot be reached is not one to start here.
    choose('DeepSeek');
    shell.check.mockResolvedValueOnce({ error: 'the service could not be reached', kind: 'network' });
    await check();
    expect(one('key-checked').textContent).toContain('The service could not be reached.');
  });

  it('says nothing of a check that another service overtook', async () => {
    const shell = await opened();
    const late = held<unknown>();
    shell.check.mockReturnValueOnce(late.promise);
    await act(async () => { fireEvent.click(one('key-check')); });
    expect(one('key-checking').textContent).toBe('Checking');
    expect((one('key-check') as HTMLButtonElement).disabled).toBe(true);
    choose('OpenAI');
    expect(marked('key-checking')).toEqual([]);
    await act(async () => { late.release({ models: ['deepseek-flash'] }); });
    await settled();
    expect(marked('key-checked')).toEqual([]);
  });

  it('marks a model that looks like a key, and a key with a blank inside it, which is neither checked nor stored', async () => {
    await opened();
    write('key-model', 'sk-proj-0123456789');
    expect(one('key-like-a-key').textContent).toBe('This looks like a key, and not like the name of a model.');
    write('key-model', 'deepseek-flash');
    expect(marked('key-like-a-key')).toEqual([]);
    write('key-key', 'sk-one two');
    expect(one('key-blank').textContent).toBe('A key has no blank in it. See whether what was pasted brought more along.');
    expect((one('key-check') as HTMLButtonElement).disabled).toBe(true);
    expect((one('key-store') as HTMLButtonElement).disabled).toBe(true);
    write('key-key', '  sk-one-two  ');
    expect(marked('key-blank')).toEqual([]);
    expect((one('key-check') as HTMLButtonElement).disabled).toBe(false);
  });

  it('deletes a stored key once it was asked whether to, and says so', async () => {
    const shell = await opened({ service: 'openai' });
    expect(marked('key-forget')).toHaveLength(1);
    fireEvent.click(one('key-forget'));
    expect(one('key-forget-ask').textContent).toContain(
      'Delete the key that is stored for OpenAI? What else is set up for it stays.',
    );
    // Keeping it asks nothing of the shell.
    fireEvent.click(one('key-forget-keep'));
    expect(marked('key-forget-ask')).toEqual([]);
    expect(shell.forget).not.toHaveBeenCalled();

    const after = listed();
    after.services[1] = { ...after.services[1], key: 'none' };
    shell.context.mockResolvedValueOnce({ service: 'openai', language: 'en', theme: 'moonveil', list: after });
    fireEvent.click(one('key-forget'));
    await act(async () => { fireEvent.click(one('key-forget-sure')); });
    await settled();
    expect(shell.forget).toHaveBeenCalledExactlyOnceWith('openai');
    expect(one('key-forgot').textContent).toBe('The key of OpenAI was deleted.');
    // The list says anew where the key comes from: there is none to delete any more.
    expect(one('key-note').textContent).toBe('No key is there for this service yet.');
    expect(marked('key-forget')).toEqual([]);
  });

  it('says so where no key was there to delete, or the program refused', async () => {
    const shell = await opened({ service: 'openai' });
    shell.forget.mockResolvedValueOnce({ forgotten: { name: 'openai', key: false, chosen: true } });
    fireEvent.click(one('key-forget'));
    await act(async () => { fireEvent.click(one('key-forget-sure')); });
    await settled();
    expect(one('key-forgot').textContent).toBe('No key was stored for OpenAI.');

    shell.forget.mockResolvedValueOnce({ error: 'the keyring did not answer' });
    fireEvent.click(one('key-forget'));
    await act(async () => { fireEvent.click(one('key-forget-sure')); });
    await settled();
    expect(one('key-forgot').textContent).toBe('The keyring did not answer.');
    expect(one('key-forgot').dataset.tone).toBe('bad');

    shell.forget.mockRejectedValueOnce(new Error('That service is not among those that are set up.'));
    fireEvent.click(one('key-forget'));
    await act(async () => { fireEvent.click(one('key-forget-sure')); });
    await settled();
    expect(one('key-forgot').textContent).toBe('That service is not among those that are set up.');
  });

  it('offers no deletion for a service without a stored key', async () => {
    await opened();
    expect(marked('key-forget')).toEqual([]);
    choose('lab');
    expect(marked('key-forget')).toEqual([]);
  });
});
