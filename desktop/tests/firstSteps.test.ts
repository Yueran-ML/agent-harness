import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Account } from '../src/app/github';
import {
  firstStepsOf, gitHubStepOf, modelStepOf, PROFILE_SERVICE, type GitHubStep, type ModelStep,
} from '../src/app/firstSteps';
import type { Listed } from '../src/key/form';
import { INSTALL_URL, service } from './mounted';

const listedWith = (use: string | null, ...services: Listed['services']): Listed => ({ use, services });
const deepseek = (key = 'none') => ({ ...service('deepseek', 'DeepSeek', { key, key_env: 'DEEPSEEK_API_KEY' }), model: 'deepseek-flash' });
const octocat = { login: 'octocat', name: 'The Octocat', avatar: null };

describe('the service that the application talks to where none was chosen', () => {
  it('is the one of the profile that the application starts', () => {
    const profile = readFileSync(new URL('../../src/columbina/bundles/serve.yml', import.meta.url), 'utf8');
    const model = /\n\s+model:\n\s+provider: ([a-z-]+)\n/.exec(profile);
    expect(model?.[1]).toBe(PROFILE_SERVICE);
  });
});

describe('the step of the model service', () => {
  it('is being checked until the program has answered, and unknown where it could not be asked', () => {
    expect(modelStepOf(null, true)).toEqual({ kind: 'checking' });
    expect(modelStepOf(null, false)).toEqual({ kind: 'unknown' });
  });

  it('is that of the service of the profile where none was chosen', () => {
    expect(modelStepOf(listedWith(null, deepseek(), service('openai', 'OpenAI', { key: 'stored' })), false))
      .toEqual({ kind: 'missing', service: 'deepseek', title: 'DeepSeek' });
    expect(modelStepOf(listedWith(null, deepseek('stored')), false))
      .toEqual({ kind: 'ready', service: 'deepseek', title: 'DeepSeek', model: 'deepseek-flash' });
  });

  it('is that of the service that was chosen', () => {
    const listed = listedWith('openai', deepseek(), { ...service('openai', 'OpenAI', { key: 'environment' }), model: null });
    expect(modelStepOf(listed, false)).toEqual({ kind: 'ready', service: 'openai', title: 'OpenAI', model: null });
  });

  it('shows what is there while the program is asked again', () => {
    expect(modelStepOf(listedWith(null, deepseek('stored')), true).kind).toBe('ready');
  });

  it('is ready for a service that asks for no key, and for one of the user\'s own', () => {
    expect(modelStepOf(listedWith('ollama', service('ollama', 'Ollama', { key_env: null })), false).kind).toBe('ready');
    expect(modelStepOf(listedWith('mine', service('mine', 'http://10.0.0.5:8000/v1', { own: true })), false))
      .toEqual({ kind: 'ready', service: 'mine', title: 'mine', model: 'a-model' });
  });

  it('lacks a key that is stored for another address', () => {
    expect(modelStepOf(listedWith(null, deepseek('stored-for-another-address')), false).kind).toBe('missing');
  });

  it('is unknown where the list has no such service', () => {
    expect(modelStepOf(listedWith(null, service('openai', 'OpenAI', { key: 'stored' })), false)).toEqual({ kind: 'unknown' });
  });
});

describe('the step of GitHub', () => {
  const nobody: Account = { kind: 'nobody', installUrl: INSTALL_URL, ended: null };

  it('is being checked until the program has said who is signed in', () => {
    expect(gitHubStepOf({ kind: 'unknown' }, false)).toEqual({ kind: 'checking' });
  });

  it('is offered where nobody is signed in, unless the user said that it can wait', () => {
    expect(gitHubStepOf(nobody, false)).toEqual({ kind: 'offered' });
    expect(gitHubStepOf({ ...nobody, ended: 'the sign-in ran out' }, false)).toEqual({ kind: 'offered' });
    expect(gitHubStepOf(nobody, true)).toEqual({ kind: 'hidden' });
  });

  it('is ready where somebody is signed in, whatever the user said before', () => {
    const account: Account = { kind: 'signedIn', person: octocat, repositories: [], offline: null, installUrl: INSTALL_URL };
    for (const skipped of [false, true]) {
      expect(gitHubStepOf(account, skipped)).toEqual({ kind: 'ready', person: octocat, repositories: [], installUrl: INSTALL_URL });
    }
  });

  it('is not offered where it cannot be done from here', () => {
    for (const account of [
      { kind: 'unavailable' }, { kind: 'unregistered', why: 'no app' }, { kind: 'failed', code: 'STORE', said: 'unreadable' },
    ] as Account[]) {
      expect(gitHubStepOf(account, false)).toEqual({ kind: 'hidden' });
    }
  });
});

describe('the first steps', () => {
  const ready: ModelStep = { kind: 'ready', service: 'deepseek', title: 'DeepSeek', model: 'deepseek-flash' };
  const signedIn: GitHubStep = { kind: 'ready', person: octocat, repositories: ['octocat/hello'], installUrl: INSTALL_URL };

  it('are not shown until it is known where each stands, or where nothing can be set up from here', () => {
    expect(firstStepsOf({ kind: 'checking' }, { kind: 'offered' })).toEqual({ kind: 'none' });
    expect(firstStepsOf(ready, { kind: 'checking' })).toEqual({ kind: 'none' });
    expect(firstStepsOf({ kind: 'unknown' }, { kind: 'offered' })).toEqual({ kind: 'none' });
  });

  it('are open while the model service lacks its key, or GitHub is offered', () => {
    expect(firstStepsOf({ kind: 'missing', service: 'deepseek', title: 'DeepSeek' }, signedIn)).toEqual({ kind: 'open' });
    expect(firstStepsOf(ready, { kind: 'offered' })).toEqual({ kind: 'open' });
  });

  it('are done in one line once all that is offered is done', () => {
    expect(firstStepsOf(ready, signedIn)).toEqual({ kind: 'done', line: 'DeepSeek · deepseek-flash · @octocat' });
    expect(firstStepsOf(ready, { kind: 'hidden' })).toEqual({ kind: 'done', line: 'DeepSeek · deepseek-flash' });
    expect(firstStepsOf({ ...ready, model: null }, { kind: 'hidden' })).toEqual({ kind: 'done', line: 'DeepSeek' });
  });
});
