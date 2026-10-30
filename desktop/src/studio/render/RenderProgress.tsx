import { FolderOpen, X } from 'lucide-react';
import { useState } from 'react';
import { useWords } from '../../app/locale';
import { sentenceOf } from '../../key/form';
import { fileName } from '../timecode';
import { fellBack, type RenderJob } from './renders';

/**
 * How the export of the window goes, in a strip at the top of the studio: it lies over nothing, so
 * the user goes on editing while it runs (the engine renders the timeline as it was when it began).
 * While it runs it shows how far it is, how fast, and with which encoder, and it can be cancelled;
 * once it is done, the file can be shown in its folder; where it failed, it says why, in the words
 * of the engine; where the computer had no encoder for H.264 or HEVC, that ProRes was written instead.
 */
export function RenderProgress({ job, onCancel, onReveal, onDismiss }: {
  job: RenderJob;
  onCancel: () => void;
  onReveal: () => Promise<boolean>;
  onDismiss: () => void;
}) {
  const words = useWords();
  const labels = words.label.studio.render;
  const hints = words.hint.studio.render;
  const [unrevealed, setUnrevealed] = useState(false);
  const going = job.state === 'running' || job.state === 'cancelling';
  const part = job.frames > 0 ? Math.min(1, job.frame / job.frames) : 0;
  const said = job.state === 'done' ? hints.done
    : job.state === 'failed' ? hints.failed
      : job.state === 'cancelled' ? hints.cancelled
        : job.state === 'stopped' ? hints.stopped : null;
  const detail = job.state === 'failed' && job.message ? sentenceOf(job.message) : null;
  return <div className="render-progress" role="status" aria-label={labels.exporting} data-state={job.state} data-test="render-progress">
    <div className="render-progress-words">
      <p>
        <strong>{going ? (job.state === 'cancelling' ? labels.cancelling : labels.exporting) : said}</strong>
        <span className="render-progress-name" title={job.path ?? undefined}>{job.path ? fileName(job.path) : job.name}</span>
      </p>
      {going && <>
        <div className="render-bar" role="progressbar" aria-valuemin={0} aria-valuemax={job.frames} aria-valuenow={job.frame} data-test="render-bar">
          <span style={{ width: `${(part * 100).toFixed(1)}%` }} />
        </div>
        <p className="mono caption" data-test="render-numbers">
          {labels.frames(job.frame, job.frames)}
          {job.fps > 0 && <> · {words.label.studio.fps(job.fps.toFixed(1))}</>}
          {job.encoder && <> · <span title={labels.encoder}>{job.encoder}</span></>}
        </p>
      </>}
      {detail && <p className="caption" lang="en" title={detail} data-test="render-detail">{detail}</p>}
      {fellBack(job) && <p className="caption" data-test="render-fallback">{hints.fallback}</p>}
      {unrevealed && <p className="caption" lang="en">{job.path}</p>}
    </div>
    {going && <button type="button" className="btn btn-quiet" disabled={job.state === 'cancelling'} onClick={onCancel} data-test="render-cancel">{words.label.cancel}</button>}
    {job.state === 'done' && <button type="button" className="btn btn-quiet" onClick={() => void onReveal().then(shown => setUnrevealed(!shown), () => setUnrevealed(true))}
      data-test="render-reveal"><FolderOpen size={14} aria-hidden="true" />{labels.reveal}</button>}
    {!going && <button type="button" className="icon-btn" onClick={onDismiss} aria-label={words.label.close} title={words.label.close} data-test="render-dismiss">
      <X size={14} />
    </button>}
  </div>;
}
