import type { Messages } from '../../messages/en';
import { EngineError } from '../engine';

/** Why the engine refused a change, said in one sentence, and the longest transition that fits where it says one. */
export interface Refusal { reason: string | null; text: string; longest: number | null }

/**
 * The sentence for a refusal of the engine: by its reason (`hint.studio.edit.*`, or one that the
 * caller says better for its own panel), else the general one with what the engine said. A caller
 * may say a refusal of one thing better still: by the reason, a colon, and what the engine says the
 * refusal is of (its `what`: `out_of_range:delta_keyer.screen`), which goes before the sentence of
 * the reason alone.
 */
export function refusalOf(error: unknown, words: Messages['hint']['studio'], overrides: Partial<Record<string, string>> = {}): Refusal {
  const data = error instanceof EngineError && error.data && typeof error.data === 'object' ? error.data as { reason?: unknown; longest?: unknown; what?: unknown } : null;
  const reason = typeof data?.reason === 'string' ? data.reason : null;
  const sentences = words.edit;
  const ofWhat = reason !== null && typeof data?.what === 'string' ? overrides[`${reason}:${data.what}`] : undefined;
  const text = ofWhat ? ofWhat
    : reason !== null && overrides[reason] ? overrides[reason]!
    : reason !== null && Object.hasOwn(sentences, reason) ? sentences[reason as keyof typeof sentences]
      : `${sentences.failed}${error instanceof Error ? ` ${error.message}` : ''}`;
  const longest = reason === 'no_handles' && typeof data?.longest === 'number' && data.longest > 0 ? data.longest : null;
  return { reason, text, longest };
}

/** The refusal in a line: its sentence, and the longest transition that fits after it. */
export function refusalText(error: unknown, words: Messages['hint']['studio']): string {
  const { text, longest } = refusalOf(error, words);
  if (longest === null) return text;
  const more = words.inspector.longest(longest);
  return /[。．]$/.test(text) ? `${text}${more}` : `${text} ${more}`;
}
