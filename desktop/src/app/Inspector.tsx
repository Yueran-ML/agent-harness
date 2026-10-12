import { ChevronRight, X } from 'lucide-react';
import type { ToolItem } from '../state';
import type { Workspace } from '../useWorkspace';
import { CopyButton } from './Markdown';
import { ToolRow } from './ToolRow';
import { useWords } from './locale';
import { contextPercent } from './tell';

export type InspectorTab = 'changes' | 'details';

/** The calls of a task that change files. */
export function changesOf(workspace: Workspace): ToolItem[] {
  return workspace.projection.items.filter(
    (item): item is ToolItem => item.type === 'tool' && (item.name === 'edit' || item.name === 'write'),
  );
}

function short(count: number, locale: string): string {
  return Intl.NumberFormat(locale, { notation: 'compact', maximumFractionDigits: 1 }).format(count);
}

function Details({ workspace, locale }: { workspace: Workspace; locale: string }) {
  const words = useWords();
  const { projection, status } = workspace;
  const read = projection.usage.input_tokens + projection.usage.cache_read_tokens + projection.usage.cache_write_tokens;
  const cached = read > 0 ? Math.round(projection.usage.cache_read_tokens / read * 100) : 0;
  const percent = contextPercent(projection.lastInputTokens, status?.model.context_window);
  const used = short(projection.lastInputTokens, locale);
  const window = status?.model.context_window;

  return <div className="details">
    {status && <section>
      <div className="overline">{words.label.model}</div>
      <p className="details-main">{status.model.model}</p>
      <p className="caption">{status.model.provider}</p>
    </section>}

    <section>
      <div className="overline">{words.label.contextWindow}</div>
      {percent !== null && <div className="meter" role="img" aria-label={words.label.context(percent)}>
        <span style={{ width: `${percent}%` }} data-tone={percent >= 95 ? 'bad' : percent >= 80 ? 'warn' : undefined} />
      </div>}
      <p className="caption">
        {window ? words.hint.contextOf(used, short(window, locale)) : words.hint.contextUnknown(used)}
      </p>
    </section>

    <section>
      <div className="overline">{words.label.usage}</div>
      <dl className="figures">
        <div><dt>{words.label.usageIn}</dt><dd>{short(read, locale)}</dd></div>
        <div><dt>{words.label.usageOut}</dt><dd>{short(projection.usage.output_tokens, locale)}</dd></div>
        <div><dt>{words.label.usageCached}</dt><dd>{cached}%</dd></div>
      </dl>
    </section>

    <section>
      <div className="overline">{words.label.tools}</div>
      {projection.tools.length === 0
        ? <p className="caption">{words.hint.noToolsYet}</p>
        : projection.tools.map(tool => <details className="aside" key={tool.name}>
          <summary><ChevronRight size={13} aria-hidden="true" /><code>{tool.name}</code></summary>
          <p className="tool-about">{tool.description}</p>
        </details>)}
    </section>

    {workspace.sessionId && <section>
      <div className="overline">{words.label.taskId}</div>
      <p className="task-id"><code>{workspace.sessionId}</code><CopyButton text={workspace.sessionId} /></p>
    </section>}
  </div>;
}

/** A panel beside the conversation: what the task changed, and the figures of the task. */
export function Inspector({ workspace, tab, onTab, onClose, locale }: {
  workspace: Workspace;
  tab: InspectorTab;
  onTab: (tab: InspectorTab) => void;
  onClose: () => void;
  locale: string;
}) {
  const words = useWords();
  const changes = changesOf(workspace);
  return <aside className="inspector" aria-label={tab === 'changes' ? words.label.changes : words.label.details} data-test="inspector">
    <div className="inspector-head">
      <div className="tabs" role="tablist">
        <button type="button" role="tab" aria-selected={tab === 'changes'} onClick={() => onTab('changes')}>
          {words.label.changes}{changes.length > 0 && <span className="count">{changes.length}</span>}
        </button>
        <button type="button" role="tab" aria-selected={tab === 'details'} onClick={() => onTab('details')}>
          {words.label.details}
        </button>
      </div>
      <button type="button" className="icon-btn" onClick={onClose} aria-label={words.label.close} title={words.label.close}>
        <X size={15} />
      </button>
    </div>
    <div className="inspector-body" role="tabpanel">
      {tab === 'details' ? <Details workspace={workspace} locale={locale} />
        : changes.length === 0 ? <p className="caption inspector-none">{words.hint.noChanges}</p>
          : <div className="steps">{changes.map(item => <ToolRow key={item.id} item={item} />)}</div>}
    </div>
  </aside>;
}
