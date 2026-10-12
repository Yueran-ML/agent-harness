import { describe, expect, it } from 'vitest';
import {
  called, changedBeyondKey, chosen, complete, documentOf, formFor, listedOf, onlyTheKey, sentenceOf, serviceCalled, serviceOf,
  storedOf, takesKey, tokensOf,
  type Form, type Listed,
} from '../src/key/form';

// What `columbina setup --list` wrote in a real run, shortened to three services, and one of the user's own.
// Of that one the program knows only where it is, and gives the address as its title.
const told = {
  use: 'openai',
  services: [
    { name: 'deepseek', title: 'DeepSeek', base_url: 'https://api.deepseek.com', own: false, model: 'deepseek-flash', context_window: 1048576, key_env: 'DEEPSEEK_API_KEY', key: 'none' },
    { name: 'openai', title: 'OpenAI', base_url: 'https://api.openai.com/v1', own: false, model: 'gpt-test', context_window: 400000, key_env: 'OPENAI_API_KEY', key: 'stored' },
    { name: 'ollama', title: 'Ollama on the same machine', base_url: 'http://localhost:11434/v1', own: false, model: null, context_window: null, key_env: null, key: 'none' },
    { name: 'my-server', title: 'http://10.0.0.5:8000/v1', base_url: 'http://10.0.0.5:8000/v1', own: true, model: 'tiny', context_window: 32768, key_env: 'COLUMBINA_API_KEY', key: 'stored-for-another-address' },
  ],
};
const listed = listedOf(told) as Listed;
const form = (given: Partial<Form> = {}): Form => ({
  adding: false, service: 'openai', name: '', address: '', model: 'gpt-test', window: '', key: '', ...given,
});

describe('what the shell says is set up', () => {
  it('is read as it was told', () => {
    expect(listed).toEqual(told);
  });

  it('is nothing where it is no list of services', () => {
    for (const given of [null, undefined, 'openai', 7, [], { error: 'the file is not YAML' }, { use: 'openai' }, { services: 'openai' }]) {
      expect(listedOf(given)).toBeNull();
    }
  });

  it('holds only what is a service with a name', () => {
    const read = listedOf({ use: 7, services: [null, 'openai', 3, {}, { name: '' }, { name: 7 }, { name: 'mine' }] });
    expect(read).toEqual({
      use: null,
      services: [{ name: 'mine', title: 'mine', base_url: '', own: false, model: null, context_window: null, key_env: null, key: 'none' }],
    });
  });

  it('takes of each service only what is of the kind it should be', () => {
    const read = listedOf({ use: 'mine', services: [{
      name: 'mine', title: 7, base_url: null, own: 'yes', model: '', context_window: '32768', key_env: 0, key: null,
    }, { name: 'other', context_window: 0 }, { name: 'third', context_window: 1.5 }, { name: 'fourth', context_window: -4 }] });
    expect(read?.services.map(service => service.context_window)).toEqual([null, null, null, null]);
    expect(read?.services[0]).toEqual({
      name: 'mine', title: 'mine', base_url: '', own: false, model: null, context_window: null, key_env: null, key: 'none',
    });
  });
});

describe('what a person calls a service', () => {
  it('is its title, and the name that its user gave it for one that the user added', () => {
    expect(listed.services.map(called)).toEqual(['DeepSeek', 'OpenAI', 'Ollama on the same machine', 'my-server']);
    // A service that is known by name keeps its title, whatever its name is.
    expect(called({ ...listed.services[3], own: false })).toBe('http://10.0.0.5:8000/v1');
    expect(called({ ...listed.services[1], own: true })).toBe('openai');
  });

  it('is found by the name of the service, and is that name where the list does not have it', () => {
    expect(serviceCalled('openai', listed)).toBe('OpenAI');
    expect(serviceCalled('my-server', listed)).toBe('my-server');
    expect(serviceCalled('gone', listed)).toBe('gone');
    expect(serviceCalled('openai', null)).toBe('openai');
    // Not the first of the list, and not one whose name only begins alike.
    expect(serviceCalled('open', listed)).toBe('open');
  });
});

describe('the form as the window opens with it', () => {
  it('is about the service that the window was opened for', () => {
    expect(formFor(listed, 'deepseek')).toEqual(form({ service: 'deepseek', model: 'deepseek-flash', window: '1048576' }));
  });

  it('is about the service in use where none was asked for, or one that is not there', () => {
    expect(formFor(listed, null).service).toBe('openai');
    expect(formFor(listed, 'gone').service).toBe('openai');
    expect(formFor(listed, null)).toEqual(form({ window: '400000' }));
  });

  it('is about the first service where none is in use', () => {
    expect(formFor({ ...listed, use: null }, null).service).toBe('deepseek');
    expect(formFor({ ...listed, use: 'gone' }, null).service).toBe('deepseek');
  });

  it('adds a service where there is none at all', () => {
    expect(formFor({ use: null, services: [] }, null)).toEqual(form({ adding: true, service: '', model: '' }));
  });

  it('leaves empty what is not known of a service', () => {
    expect(formFor(listed, 'ollama')).toEqual(form({ service: 'ollama', model: '', window: '' }));
  });
});

describe('choosing another service', () => {
  it('puts what is known of it in the place of what stood there', () => {
    const before = form({ model: 'typed-by-hand', window: '99', key: 'sk-made-up' });
    expect(chosen(before, listed, 'my-server')).toEqual(form({ service: 'my-server', model: 'tiny', window: '32768' }));
  });

  it('does not take the key that was typed for one service along to another', () => {
    expect(chosen(form({ key: 'sk-made-up' }), listed, 'deepseek').key).toBe('');
    expect(chosen(form({ key: 'sk-made-up' }), listed, null).key).toBe('');
  });

  it('begins a service of the user\'s own where that is chosen, or a service that is not there', () => {
    for (const service of [null, 'gone']) {
      expect(chosen(form({ model: 'gpt-test', window: '400000' }), listed, service))
        .toEqual(form({ adding: true, service: '', model: '', window: '' }));
    }
  });

  it('keeps the name and the address of a service that was begun, for when the user comes back to it', () => {
    const begun = form({ adding: true, service: '', name: 'lab', address: 'http://10.0.0.9/v1' });
    const away = chosen(begun, listed, 'openai');
    expect(chosen(away, listed, null)).toMatchObject({ adding: true, name: 'lab', address: 'http://10.0.0.9/v1' });
  });
});

describe('a form that changes nothing but the key', () => {
  // The form as the window opens with it for the service in use: its model and its window as they are known.
  const opened = formFor(listed, 'openai');

  it('is one of the service in use, with its model and its window as they are', () => {
    expect(onlyTheKey({ ...opened, key: 'sk-a-key' }, listed, null)).toBe(true);
    // Blanks around what stands there change nothing.
    expect(onlyTheKey({ ...opened, model: ' gpt-test ', window: '400 000', key: 'sk-a-key' }, listed, null)).toBe(true);
  });

  it('is none that names another model or another size of the window', () => {
    expect(onlyTheKey({ ...opened, model: 'gpt-other' }, listed, null)).toBe(false);
    expect(onlyTheKey({ ...opened, window: '128000' }, listed, null)).toBe(false);
    expect(onlyTheKey({ ...opened, window: '' }, listed, null)).toBe(false);
  });

  it('is none of another service than the one in use, or of one that is being added', () => {
    expect(onlyTheKey(formFor(listed, 'deepseek'), listed, null)).toBe(false);
    expect(onlyTheKey({ ...opened, adding: true, service: '' }, listed, null)).toBe(false);
    expect(onlyTheKey({ ...opened, service: 'gone' }, listed, null)).toBe(false);
  });

  it('goes by the service that the program talks to, where none was chosen', () => {
    const fresh = { ...listed, use: null };
    // The program goes by its profile then, and the main window says which service that is.
    expect(onlyTheKey(formFor(fresh, 'deepseek'), fresh, 'deepseek')).toBe(true);
    expect(onlyTheKey(formFor(fresh, 'openai'), fresh, 'deepseek')).toBe(false);
    expect(onlyTheKey(formFor(fresh, 'deepseek'), fresh, null)).toBe(false);
    // What was chosen goes before what the main window said.
    expect(onlyTheKey(formFor(listed, 'deepseek'), listed, 'deepseek')).toBe(false);
  });

  it('is one of a service that has no model or window yet, as long as none is given', () => {
    const local = { ...listed, use: 'ollama' };
    // A form cannot be sent without a model, so this one would not be. It changes nothing all the same.
    expect(onlyTheKey(formFor(local, 'ollama'), local, null)).toBe(true);
    expect(onlyTheKey({ ...formFor(local, 'ollama'), model: 'llama3' }, local, null)).toBe(false);
  });
});

describe('what was stored beside a key', () => {
  const after = (change: (services: Listed['services']) => void, use: string | null = listed.use): Listed => {
    const copy: Listed = JSON.parse(JSON.stringify(listed));
    copy.use = use;
    change(copy.services);
    return copy;
  };
  const of = (services: Listed['services'], name: string) => services.find(service => service.name === name)!;

  it('is nothing where only the key of the service in use is another', () => {
    expect(changedBeyondKey(listed, after((services) => { of(services, 'openai').key = 'environment'; }), null)).toBe(false);
    // Nor where a key was stored for a service that is not in use.
    expect(changedBeyondKey(listed, after((services) => { of(services, 'deepseek').key = 'stored'; }), null)).toBe(false);
    expect(changedBeyondKey(listed, after(() => {}), null)).toBe(false);
  });

  it('is another service', () => {
    expect(changedBeyondKey(listed, after(() => {}, 'deepseek'), null)).toBe(true);
    expect(changedBeyondKey(listed, after(() => {}, null), null)).toBe(true);
  });

  it('is another address, model or window of the service in use', () => {
    expect(changedBeyondKey(listed, after((services) => { of(services, 'openai').base_url = 'https://proxy.example.com/v1'; }), null)).toBe(true);
    expect(changedBeyondKey(listed, after((services) => { of(services, 'openai').model = 'gpt-other'; }), null)).toBe(true);
    expect(changedBeyondKey(listed, after((services) => { of(services, 'openai').context_window = 128000; }), null)).toBe(true);
    // The same of a service that is not in use changes nothing for the program that runs.
    expect(changedBeyondKey(listed, after((services) => { of(services, 'deepseek').model = 'deepseek-other'; }), null)).toBe(false);
  });

  it('is measured against the service that the program talks to, where none was chosen before', () => {
    const fresh = { ...listed, use: null };
    // A first key for the service of the profile: the choice is written down, and nothing changes for the program.
    expect(changedBeyondKey(fresh, after((services) => { of(services, 'deepseek').key = 'stored'; }, 'deepseek'), 'deepseek')).toBe(false);
    expect(changedBeyondKey(fresh, after(() => {}, 'openai'), 'deepseek')).toBe(true);
    expect(changedBeyondKey(fresh, after((services) => { of(services, 'deepseek').model = 'deepseek-other'; }, 'deepseek'), 'deepseek')).toBe(true);
    // Nobody knows which service the program talks to: it cannot be ruled out that this is another.
    expect(changedBeyondKey(fresh, after(() => {}, 'deepseek'), null)).toBe(true);
    expect(changedBeyondKey(fresh, after(() => {}, null), null)).toBe(true);
    // What was chosen before goes before what the main window says of the program that runs.
    expect(changedBeyondKey(listed, after((services) => { of(services, 'openai').key = 'environment'; }), 'deepseek')).toBe(false);
  });

  it('cannot be ruled out where a list is not known, or does not have the service', () => {
    expect(changedBeyondKey(null, listed, 'openai')).toBe(true);
    expect(changedBeyondKey(listed, null, 'openai')).toBe(true);
    expect(changedBeyondKey(listed, after((services) => { services.splice(1, 1); }), null)).toBe(true);
    expect(changedBeyondKey({ ...listed, use: 'gone' }, { ...listed, use: 'gone' }, null)).toBe(true);
  });
});

describe('the service of a form', () => {
  it('is the listed one, and none while one is added', () => {
    expect(serviceOf(form(), listed)?.title).toBe('OpenAI');
    expect(serviceOf(form({ adding: true, service: 'openai' }), listed)).toBeNull();
    expect(serviceOf(form({ service: 'gone' }), listed)).toBeNull();
  });

  it('takes a key unless it is one that asks for none', () => {
    expect(takesKey(form(), listed)).toBe(true);
    expect(takesKey(form({ service: 'ollama' }), listed)).toBe(false);
    // One that is being added may ask for one.
    expect(takesKey(form({ adding: true, service: '' }), listed)).toBe(true);
  });
});

describe('a number of tokens as it is typed', () => {
  it('is none where nothing is typed', () => {
    expect(tokensOf('')).toBeNull();
    expect(tokensOf('   ')).toBeNull();
  });

  it('is read with the marks that people group digits with', () => {
    expect(tokensOf('128000')).toBe(128000);
    expect(tokensOf(' 128,000 ')).toBe(128000);
    expect(tokensOf('128 000')).toBe(128000);
    expect(tokensOf('1_048_576')).toBe(1048576);
    expect(tokensOf('1')).toBe(1);
  });

  it('is no number where what is typed is none of tokens', () => {
    for (const typed of ['0', '-5', '1.5', 'abc', '12k', '1e6', '0128', '+7', '１２８', '1'.repeat(13)]) {
      expect(tokensOf(typed)).toBeNaN();
    }
  });
});

describe('a form that can be sent', () => {
  it('names a model', () => {
    expect(complete(form())).toBe(true);
    expect(complete(form({ model: '' }))).toBe(false);
    expect(complete(form({ model: '   ' }))).toBe(false);
  });

  it('has a number of tokens or none, and nothing else in that place', () => {
    expect(complete(form({ window: '128,000' }))).toBe(true);
    expect(complete(form({ window: 'many' }))).toBe(false);
  });

  it('names the service that it adds and says where it is', () => {
    const adding = form({ adding: true, service: '', name: 'lab', address: 'http://10.0.0.9/v1' });
    expect(complete(adding)).toBe(true);
    expect(complete({ ...adding, name: ' ' })).toBe(false);
    expect(complete({ ...adding, address: '' })).toBe(false);
    // A listed service needs neither.
    expect(complete(form({ name: '', address: '' }))).toBe(true);
  });

  it('needs no key', () => {
    // A key that is stored stays, and a service may ask for none.
    expect(complete(form({ key: '' }))).toBe(true);
  });
});

describe('what is sent for a form', () => {
  it('is the service and the model, and nothing that was not said', () => {
    expect(documentOf(form(), listed)).toEqual({ service: 'openai', model: 'gpt-test' });
  });

  it('holds the size and the key where they were typed, without the blanks around them', () => {
    expect(documentOf(form({ model: ' gpt-test ', window: ' 400,000 ', key: '  sk-made-up-0123456789\n' }), listed)).toEqual({
      service: 'openai', model: 'gpt-test', context_window: 400000, api_key: 'sk-made-up-0123456789',
    });
  });

  it('holds no address for a service that is listed, whatever stands in the field of one', () => {
    expect(documentOf(form({ name: 'lab', address: 'http://10.0.0.9/v1' }), listed)).toEqual({ service: 'openai', model: 'gpt-test' });
  });

  it('holds the name and the address of a service that is added', () => {
    const adding = form({ adding: true, service: 'openai', name: ' lab ', address: ' http://10.0.0.9/v1 ', model: 'tiny' });
    expect(documentOf(adding, listed)).toEqual({ service: 'lab', base_url: 'http://10.0.0.9/v1', model: 'tiny' });
  });

  it('holds no key for a service that asks for none', () => {
    expect(documentOf(form({ service: 'ollama', model: 'llama', key: 'sk-made-up' }), listed)).toEqual({ service: 'ollama', model: 'llama' });
  });

  it('holds no size where what was typed is no number of tokens', () => {
    expect(documentOf(form({ window: 'many' }), listed)).toEqual({ service: 'openai', model: 'gpt-test' });
  });

  it('holds no key that is nothing but blanks', () => {
    expect(documentOf(form({ key: '   ' }), listed)).toEqual({ service: 'openai', model: 'gpt-test' });
  });
});

describe('a sentence that the program wrote', () => {
  it('is shown with a capital at its beginning and a full stop at its end', () => {
    expect(sentenceOf('the document names no model')).toBe('The document names no model.');
    expect(sentenceOf('  the file provider.yml is not YAML  ')).toBe('The file provider.yml is not YAML.');
  });

  it('keeps the end that it has', () => {
    expect(sentenceOf('Nothing of it was kept.')).toBe('Nothing of it was kept.');
    expect(sentenceOf('is it there?')).toBe('Is it there?');
    expect(sentenceOf('没有这个文件。')).toBe('没有这个文件。');
  });

  it('is nothing where the program wrote nothing', () => {
    expect(sentenceOf('')).toBe('');
    expect(sentenceOf('  ')).toBe('');
  });
});

describe('what the program answered', () => {
  const stored = { name: 'openai', base_url: 'https://api.openai.com/v1', model: 'gpt-test', context_window: 400000 };

  it('is what it stored, and where the key went', () => {
    expect(storedOf({ stored, key_in: 'keyring' })).toEqual({ kind: 'stored', name: 'openai', model: 'gpt-test', keyIn: 'keyring' });
    expect(storedOf({ stored, key_in: 'file' })).toEqual({ kind: 'stored', name: 'openai', model: 'gpt-test', keyIn: 'file' });
    expect(storedOf({ stored, key_in: null })).toEqual({ kind: 'stored', name: 'openai', model: 'gpt-test', keyIn: null });
  });

  it('says of the key only what it knows: a place that it does not know is none', () => {
    expect(storedOf({ stored, key_in: 'cloud' })).toMatchObject({ keyIn: null });
    expect(storedOf({ stored })).toMatchObject({ keyIn: null });
  });

  it('is what the program says is wrong', () => {
    expect(storedOf({ error: 'the document names no model' })).toEqual({ kind: 'refused', said: 'the document names no model' });
  });

  it('is a refusal without words where it is neither', () => {
    for (const answer of [null, undefined, 'stored', 7, [], {}, { stored: null }, { stored: 'yes' }, { stored: { name: 'openai' } }, { stored: { name: 7, model: 'x' } }, { error: 7 }]) {
      expect(storedOf(answer)).toEqual({ kind: 'refused', said: '' });
    }
  });
});
