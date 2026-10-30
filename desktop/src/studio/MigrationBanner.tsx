import { useState } from 'react';
import { X } from 'lucide-react';
import { useWords } from '../app/locale';
import { sentenceOf } from '../key/form';
import type { Timeline } from './engine';
import { folderName, type Migrated } from './migration';

/**
 * What the studio says once of the old projects of the folder that became timelines as it opened:
 * how many, and which, and that their files are kept as they were; and how many could not be
 * migrated. Its details say, of each, the timeline that it became and what of it was left out, or
 * why it was not migrated, in the words of the migration.
 */
export function MigrationBanner({ migrated, timelines, onDismiss }: {
  migrated: Migrated;
  timelines: readonly Timeline[];
  onDismiss: () => void;
}) {
  const words = useWords();
  const labels = words.label.studio.migration;
  const hints = words.hint.studio.migration;
  const [open, setOpen] = useState(false);
  // A timeline is shown by its name where the copy of the project has it, else by the folder it was made of.
  const nameOf = (id: string | undefined, source: string) => timelines.find(each => each.id === id)?.name ?? folderName(source);
  const names = migrated.done.map(report => nameOf(report.timeline, report.source));
  const detailed = migrated.done.length > 0 || migrated.failed.length > 0;
  return <div className="studio-migration" role="status" data-test="migration-banner">
    <div className="studio-migration-words">
      {names.length > 0 && <p data-test="migration-done">{hints.done(names)}</p>}
      {migrated.failed.length > 0 && <p data-test="migration-failed">{hints.failed(migrated.failed.length)}</p>}
      {migrated.error !== null && <>
        <p data-test="migration-error">{hints.checkFailed}</p>
        <p className="caption" lang="en" title={migrated.error}>{sentenceOf(migrated.error)}</p>
      </>}
      {open && <ul className="studio-migration-details" data-test="migration-details">
        {migrated.done.map(report => <li key={report.source} data-source={report.source}>
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
        {migrated.failed.map(report => <li key={report.source} data-source={report.source} data-failed="true">
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
