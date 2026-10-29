import { useWords } from '../../app/locale';

/** The Cut page holds its place among the pages, between Media and Edit, until it is made. */
export function CutPage() {
  const words = useWords();
  return <div className="studio-body cut-page" data-test="cut-page">
    <p className="caption cut-page-words">{words.hint.studio.cut}</p>
  </div>;
}
