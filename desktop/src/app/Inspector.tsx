import { ChevronRight, X } from 'lucide-react';
import type { ToolItem } from '../state';
import type { Workspace } from '../useWorkspace';
import { CopyButton } from './Markdown';
import { ToolRow } from './ToolRow';
import { useWords } from './locale';
import { contextPercent, countOf, stepped } from './tell';

export type InspectorTab = 'changes' | 'details';
const tabs: readonly InspectorTab[] = ['changes', 'details'];

/** The calls of a task that change files. */
export function changesOf(workspace: Workspace): ToolItem[] {
  return workspace.projection.items.filter(
    (item): item is ToolItem => item.type === 'tool' && (item.name === 'edit' || item.name === 'write'),
  );
}

function Details({ workspace, locale }: { workspace: Workspace; locale: string }) {
  const words = useWords();
  const { projection, status } = workspace;
  const read = projection.usage.input_tokens + projection.usage.cache_read_tokens + projection.usage.cache_write_tokens;
  const cached = read > 0 ? Math.round(projection.usage.cache_read_tokens / read * 100) : 0;
  const percent = contextPercent(projection.lastInputTokens, status?.model.context_window);
  const used = countOf(projection.lastInputTokens, locale);
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
        {window ? words.hint.contextOf(used, countOf(window, locale)) : words.hint.contextUnknown(used)}
      </p>
    </section>

    <section>
      <div className="overline">{words.label.usage}</div>
      <dl className="figures">
        <div><dt>{words.label.usageIn}</dt><dd>{countOf(read, locale)}</dd></div>
        <div><dt>{words.label.usageOut}</dt><dd>{countOf(projection.usage.output_tokens, locale)}</dd></div>
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
  const names = { changes: words.label.changes, details: words.label.details };
  return <aside className="inspector" aria-label={names[tab]} data-test="inspector">
    <div className="inspector-head">
      {/* Tabs as a keyboard knows them: one of them takes the Tab key, and the arrows go from one to the other. */}
      <div
        className="tabs"
        role="tablist"
        onKeyDown={event => {
          const to = stepped(tabs, tab, event.key);
          if (to === null || event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return;
          event.preventDefault();
          onTab(to);
          document.getElementById(`inspector-tab-${to}`)?.focus();
        }}
      >
        {tabs.map(name => <button
          type="button"
          role="tab"
          key={name}
          id={`inspector-tab-${name}`}
          aria-selected={tab === name}
          aria-controls="inspector-panel"
          tabIndex={tab === name ? 0 : -1}
          onClick={() => onTab(name)}
        >
          {names[name]}{name === 'changes' && changes.length > 0 && <span className="count">{changes.length}</span>}
        </button>)}
      </div>
      <button type="button" className="icon-btn" onClick={onClose} aria-label={words.label.close} title={words.label.close}>
        <X size={15} />
      </button>
    </div>
    <div className="inspector-body" role="tabpanel" id="inspector-panel" aria-labelledby={`inspector-tab-${tab}`}>
      {tab === 'details' ? <Details workspace={workspace} locale={locale} />
        : changes.length === 0 ? <p className="caption inspector-none">{words.hint.noChanges}</p>
          : <div className="steps">{changes.map(item => <ToolRow key={item.id} item={item} />)}</div>}
    </div>
  </aside>;
}
