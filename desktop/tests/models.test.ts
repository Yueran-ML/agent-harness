import { describe, expect, it } from 'vitest';
import { filtered, MODELS_FRESH_MS, typedModel } from '../src/app/models';

const offered = ['deepseek-flash', 'deepseek-pro', 'qwen/qwen3-coder', 'openai/gpt-pro', 'Pro'];

describe('a filter of the models', () => {
  it('keeps all of them, in the order of the service, where nothing is written', () => {
    expect(filtered(offered, '')).toEqual(offered);
    expect(filtered(offered, '   ')).toEqual(offered);
    // A new list each time, so that what the service gave is left as it was.
    expect(filtered(offered, '')).not.toBe(offered);
  });

  it('puts the one that is written first, then those that begin with it, then the others that hold it', () => {
    expect(filtered(offered, 'pro')).toEqual(['Pro', 'deepseek-pro', 'openai/gpt-pro']);
    expect(filtered(offered, 'deepseek')).toEqual(['deepseek-flash', 'deepseek-pro']);
    expect(filtered(offered, 'qwen')).toEqual(['qwen/qwen3-coder']);
    expect(filtered(offered, ' COD ')).toEqual(['qwen/qwen3-coder']);
    expect(filtered(offered, 'gemini')).toEqual([]);
    // Each kind in its place, whatever the order in which the service gave them.
    expect(filtered(['gpt-pro', 'pro-max', 'deepseek-pro', 'pro'], 'pro')).toEqual(['pro', 'pro-max', 'gpt-pro', 'deepseek-pro']);
  });
});

describe('what is typed into the filter, as a model of its own', () => {
  it('is offered where the service does not list it', () => {
    expect(typedModel(offered, 'deepseek-old')).toBe('deepseek-old');
    expect(typedModel(offered, '  anthropic/claude-sonnet-4:beta ')).toBe('anthropic/claude-sonnet-4:beta');
    expect(typedModel([], 'qwen2.5:7b')).toBe('qwen2.5:7b');
    expect(typedModel([], `model-${'x'.repeat(194)}`)).toBe(`model-${'x'.repeat(194)}`);
  });

  it('is not offered where the service lists it, in whatever case', () => {
    expect(typedModel(offered, 'deepseek-pro')).toBeNull();
    expect(typedModel(offered, 'DeepSeek-Pro')).toBeNull();
    expect(typedModel(offered, 'pro')).toBeNull();
  });

  it('is not offered where it is no name of a model, or looks like a key', () => {
    for (const typed of [
      '', '   ', 'two words', 'tab\there', 'bell\u0007', 'x'.repeat(201), `model-${'x'.repeat(195)}`,
      'sk-0123456789abcdef', 'SK-abc', 'A'.repeat(40), 'a_b'.repeat(14),
    ]) {
      expect(typedModel([], typed), JSON.stringify(typed)).toBeNull();
    }
  });
});

describe('a list of models', () => {
  it('is taken as it is for five minutes', () => {
    expect(MODELS_FRESH_MS).toBe(300_000);
  });
});
