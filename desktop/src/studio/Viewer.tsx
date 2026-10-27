import { useRef } from 'react';
import { RotateCw } from 'lucide-react';
import { useWords } from '../app/locale';
import { useViewerRect, type ViewBridge } from './useViewerRect';

/** What the viewer says in its place, where no picture can be shown there. */
export type ViewerNotice =
  | { kind: 'failed'; missing: boolean; detail: string }
  | { kind: 'takingOver' }
  | { kind: 'opening' }
  | { kind: 'stopped' }
  | { kind: 'unsupported' }
  | { kind: 'headless' }
  | { kind: 'browser' };

/**
 * The place of the picture. The page draws nothing in it: the engine lays its view over it, in
 * the desktop app. In a browser it is a grey box that says where the preview appears, and where
 * no view can be had it says why.
 */
export function Viewer({ bridge, native, active, notice, onRetry }: {
  bridge: ViewBridge;
  /** Whether the shell lays the engine's view over the viewer. */
  native: boolean;
  /** Whether the viewer is shown: the view is hidden once it is not. */
  active: boolean;
  notice: ViewerNotice | null;
  /** Open the project in the studio again. */
  onRetry: () => void;
}) {
  const words = useWords();
  const box = useRef<HTMLDivElement>(null);
  useViewerRect(box, native ? bridge : null, active && native);
  const said = words.hint.studio;
  const sentence = notice === null ? null
    : notice.kind === 'failed' ? (notice.missing ? said.missing : said.notOpened)
      : said[notice.kind];
  const retry = notice?.kind === 'failed' || notice?.kind === 'stopped';
  return <div className="viewer" data-browser={native ? undefined : ''} data-test="viewer">
    <div ref={box} className="viewer-box" data-test="viewer-box">
      {notice && <div className="viewer-notice" role="status" data-kind={notice.kind} data-test="viewer-notice">
        {(notice.kind === 'opening' || notice.kind === 'takingOver') && <span className="moon" aria-hidden="true" />}
        <p data-test="viewer-words">{sentence}</p>
        {notice.kind === 'failed' && notice.missing && <p className="viewer-install" data-test="viewer-install">{said.install}</p>}
        {/* What the shell said, in its own words, beneath what the page says of it. */}
        {notice.kind === 'failed' && notice.detail && <p className="caption viewer-detail" lang="en" data-test="viewer-detail">{notice.detail}</p>}
        {retry && <button type="button" className="btn" onClick={onRetry} data-test="retry">
          <RotateCw size={14} aria-hidden="true" />{words.label.studio.retry}
        </button>}
      </div>}
    </div>
  </div>;
}
