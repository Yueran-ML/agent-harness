import { useEffect, useRef, useState } from 'react';
import { X } from 'lucide-react';
import { useWords } from '../app/locale';
import { sentenceOf } from '../key/form';
import type { Timeline } from './engine';
import { folderName, type Migrated } from './migration';

/** How many timelines the sentence names; it counts the rest. */
export const NAMED_MOST = 3;

/**
 * What the studio says once of the old projects of the folder that became timelines as it opened:
 * how many, and which, and that their files are kept as they were; and how many could not be
 * migrated. Its details say, of each, the timeline that it became and what of it was left out, or
 * why it was not migrated, in the words of the migration.
 */
export function MigrationBanner({ migrated, timelines, onDismiss, onAnswer }: {
  migrated: Migrated;
  timelines: readonly Timeline[];
  onDismiss: () => void;
  onAnswer?: (timeline: string, accept: boolean) => Promise<void>;
}) {
  const words = useWords();
  const labels = words.label.studio.migration;
  const hints = words.hint.studio.migration;
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState<string[]>([]), [errors, setErrors] = useState<Record<string,string>>({});
  const mounted = useRef(true), inFlight = useRef(new Set<string>());
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const prompts = migrated.done.filter((report, index, all) => report.timeline && report.duck_pending && !report.duck_prompted && all.findIndex(other => other.timeline === report.timeline) === index);
  const answer = async (timeline: string, accept: boolean) => {
    if (!onAnswer || inFlight.current.has(timeline)) return;
    inFlight.current.add(timeline); setPending([...inFlight.current]); setErrors(old => ({...old,[timeline]:''}));
    try { await onAnswer(timeline, accept); }
    catch (error) { if (mounted.current) setErrors(old => ({...old,[timeline]:error instanceof Error ? error.message : String(error)})); }
    finally { inFlight.current.delete(timeline); if (mounted.current) setPending([...inFlight.current]); }
  };
  // A timeline is shown by its name where the copy of the project has it, else by the folder it was made of.
  const nameOf = (id: string | undefined, source: string) => timelines.find(each => each.id === id)?.name ?? folderName(source);
  const names = migrated.done.filter(report => !report.existing).map(report => nameOf(report.timeline, report.source));
  // The sentence names a few; the details name them all.
  const detailed = migrated.done.length > 0 || migrated.failed.length > 0;
  return <div className="studio-migration" role="status" data-test="migration-banner">
    <div className="studio-migration-words">
      {names.length > 0 && <p data-test="migration-done">{hints.done(names.slice(0, NAMED_MOST), names.length)}</p>}
      {migrated.failed.length > 0 && <p data-test="migration-failed">{hints.failed(migrated.failed.length)}</p>}
      {migrated.error !== null && <>
        <p data-test="migration-error">{hints.checkFailed}</p>
        <p className="caption" lang="en" title={migrated.error}>{sentenceOf(migrated.error)}</p>
      </>}
      {prompts.map(report => <div className="studio-migration-duck" key={report.timeline} data-duck-timeline={report.timeline}>
        <p><strong>{nameOf(report.timeline, report.source)}</strong> {hints.duck}</p>
        <button type="button" className="btn btn-quiet" disabled={!onAnswer || pending.includes(report.timeline!)} onClick={() => void answer(report.timeline!, true)}>{pending.includes(report.timeline!) ? labels.duckPending : labels.duckYes}</button>
        <button type="button" className="btn btn-quiet" disabled={!onAnswer || pending.includes(report.timeline!)} onClick={() => void answer(report.timeline!, false)}>{labels.duckNo}</button>
        {errors[report.timeline!] && <p role="alert">{hints.duckFailed} <span lang="en">{sentenceOf(errors[report.timeline!])}</span></p>}
      </div>)}
      {open && <ul className="studio-migration-details" data-test="migration-details">
        {migrated.done.map((report, at) => <li key={`${at} ${report.source}`} data-source={report.source}>
          <p><span className="setting-name">{labels.timeline}</span> <strong>{nameOf(report.timeline, report.source)}</strong>
            <span className="caption" title={report.source}> {report.source}</span></p>
          {(report.dropped ?? []).length > 0 && <>
            <p className="setting-name">{labels.dropped}</p>
            <ul>{report.dropped!.map((line, at) => <li key={at} className="caption" lang="en">{line}</li>)}</ul>
          </>}
          {(report.warnings ?? []).length > 0 && <>
            <p className="setting-name">{labels.notes}</p>
            <ul>{report.warnings!.map((line, at) => <li key={at} className="caption" lang="en">{line}</li>)}</ul>
          </>}
        </li>)}
        {migrated.failed.map((report, at) => <li key={`${at} ${report.source}`} data-source={report.source} data-failed="true">
          <p><span className="setting-name">{labels.failed}</span> <strong>{folderName(report.source)}</strong>
            <span className="caption" title={report.source}> {report.source}</span></p>
          <p className="caption" lang="en">{sentenceOf(report.error ?? '')}</p>
        </li>)}
      </ul>}
    </div>
    {detailed && <button type="button" className="btn btn-quiet" aria-expanded={open} onClick={() => setOpen(shown => !shown)} data-test="migration-toggle">
      {open ? labels.hideDetails : labels.details}
    </button>}
    <button type="button" className="icon-btn" onClick={onDismiss} aria-label={words.label.close} title={words.label.close} data-test="migration-close">
      <X size={14} />
    </button>
  </div>;
}
