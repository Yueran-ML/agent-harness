import { useCallback } from 'react';
import { useWords } from '../../app/locale';
import type { FairlightText } from '../../messages/fairlight';
import { refusalText } from '../inspector/refusal';
export function useFairlightText() {
  const text = useWords().label.studio.fairlightPage;
  return (key: FairlightText) => text[key];
}
/**
 * A failure as it is shown: the sentence for the engine's refusal in the reader's language (the
 * general one, with what the engine said, where there is none), never a bare `String(error)`.
 * The failure itself is logged for whoever looks into it.
 */
export function useFailureText() {
  const words = useWords().hint.studio;
  return useCallback((error: unknown) => {
    console.warn('[fairlight]', error);
    return refusalText(error, words);
  }, [words]);
}
