import { useWords } from '../../app/locale';
import type { FairlightText } from '../../messages/fairlight';
export function useFairlightText() {
  const text = useWords().label.studio.fairlightPage;
  return (key: FairlightText) => text[key];
}
