import { useEffect, useRef, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { ArrowDown, ArrowUpRight, Check, ChevronRight, Clock3, Code2, Copy, FileCode2, GitBranch, LoaderCircle, Moon, Sparkles, Square, Terminal, X } from 'lucide-react';
import type { ConversationItem, ToolItem } from './state';
import { messageText } from './state';
import type { Workspace } from './useWorkspace';

export function Markdown({ text, onLink }: { text: string; onLink: (url: string) => void }) {
  return <div className="markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml components={{
    a: ({ href, children }) => <button className="text-link" onClick={() => href && onLink(href)}>{children}<ArrowUpRight size={12} /></button>,
    img: ({ alt }) => <span className="media-placeholder">[Image: {alt || 'external image'}]</span>,
    pre: ({ children }) => <pre tabIndex={0}>{children}</pre>,
  }}>{text}</ReactMarkdown></div>;
}

function parseArguments(value: string): Record<string, unknown> | null {
  try { const parsed = JSON.parse(value); return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : null; }
  catch { return null; }
}

function ToolCard({ item }: { item: ToolItem }) {
  const args = parseArguments(item.arguments);
  const target = args ? String(args.command ?? args.path ?? args.pattern ?? '') : '';
  const running = item.result === null && !item.interrupted;
  const Icon = /bash|powershell|shell/.test(item.name) ? Terminal : FileCode2;
  return <details className={`tool-card ${item.error ? 'tool-error' : ''}`}>
    <summary>
      <span className="tool-icon"><Icon size={15} /></span>
      <span className="tool-label">{item.name}<code title={target}>{target || 'Tool call'}</code></span>
      <span className="tool-outcome">{running ? <LoaderCircle size={13} className="spin" /> : item.interrupted ? <><Square size={12} /><span>Interrupted</span></> : item.error ? <X size={14} /> : <Check size={14} />}
        {item.durationMs !== null && <span>{(item.durationMs / 1000).toFixed(1)}s</span>}</span>
      <ChevronRight className="disclosure" size={14} />
    </summary>
    <div className="tool-body">
      {item.name === 'edit' && args && typeof args.old === 'string' && typeof args.new === 'string' ? <div className="diff">
        <div className="code-label">{item.interrupted ? 'Interrupted change' : item.error ? 'Failed change' : item.result === null ? 'Proposed change' : 'Applied change'}</div>
        <pre className="diff-remove">{args.old.split('\n').map(line => `− ${line}`).join('\n')}</pre>
        <pre className="diff-add">{args.new.split('\n').map(line => `+ ${line}`).join('\n')}</pre>
      </div> : <><div className="code-label">Arguments</div><pre>{args ? JSON.stringify(args, null, 2) : item.arguments}</pre></>}
      {item.result !== null && <><div className="code-label">Result {item.trimmed && <span>· original output retained</span>}</div><pre className="tool-result">{item.result || '(No output)'}</pre></>}
      {item.error && <p className="error-text">{item.error.code}: {item.error.reason}</p>}
      {running && <p className="muted">Waiting for the tool to finish…</p>}
      {item.interrupted && <p className="muted">The turn ended before this operation recorded a result.</p>}
    </div>
  </details>;
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const [failed, setFailed] = useState(false);
  return <button className="icon-button copy-button" aria-label={copied ? 'Copied' : 'Copy response'} title={failed ? 'Could not copy. Select the text to copy it.' : 'Copy response'} onClick={() => {
    void navigator.clipboard.writeText(text).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1800); }).catch(() => setFailed(true));
  }}>{copied ? <Check size={14} /> : <Copy size={14} />}{failed && <span>Copy unavailable</span>}</button>;
}

function Item({ item, onLink }: { item: ConversationItem; onLink: (url: string) => void }) {
  if (item.type === 'tool') return <ToolCard item={item} />;
  if (item.type === 'user') return <article className={`message user-message ${item.source === 'steer' ? 'steer-message' : ''}`}>
    <div className="message-heading"><span className="avatar user-avatar">Y</span><strong>You</strong>{item.source === 'steer' && <span className="tag"><GitBranch size={11} /> Follow-up</span>}</div>
    {item.source === 'user' || item.source === 'steer' ? <p className="user-text">{item.text}</p> : <details><summary>Plugin context · {item.source}</summary><pre>{item.text}</pre></details>}
  </article>;
  if (item.type === 'assistant') return <article className="message assistant-message">
    <div className="message-heading"><span className="avatar agent-avatar"><Sparkles size={15} /></span><strong>Columbina</strong><span className="message-label">{item.interrupted ? 'Interrupted' : 'Assistant'}</span></div>
    {item.reasoning && <details className="reasoning"><summary><ChevronRight size={12} /> Reasoning</summary><pre>{item.reasoning}</pre></details>}
    {item.text && <Markdown text={item.text} onLink={onLink} />}
    {item.text && <CopyButton text={item.text} />}
  </article>;
  if (item.type === 'compaction') return <details className="compaction-line"><summary><Moon size={13} /> Earlier context summarized <ChevronRight size={12} /></summary><pre>{item.text}</pre><p>Earlier messages stay here. The model now receives this summary.</p></details>;
  return <div className={`notice notice-${item.tone}`} role={item.tone === 'error' ? 'alert' : undefined}><span />{item.text}</div>;
}

export function Conversation({ workspace, onLink }: { workspace: Workspace; onLink: (url: string) => void }) {
  const { projection, preview, outputs, status, loading } = workspace;
  const scroll = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [away, setAway] = useState(false);
  const hasPreview = !!(preview.text || preview.reasoning || preview.tools.length);
  const empty = projection.items.length === 0 && !hasPreview && outputs.length === 0;
  const queued = [...projection.queues['next-turn'], ...projection.queues['next-step']];
  useEffect(() => {
    if (scroll.current && empty) scroll.current.scrollTop = 0;
    else if (stick.current && scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [empty, loading, projection.items.length, preview, outputs.length, queued.length]);
  useEffect(() => { stick.current = true; setAway(false); }, [workspace.sessionId]);
  return <div className="conversation-wrap">
    <div className="conversation-scroll" ref={scroll} onScroll={() => {
      const node = scroll.current;
      if (!node) return;
      stick.current = node.scrollHeight - node.scrollTop - node.clientHeight < 100;
      setAway(!stick.current);
    }}>
      {loading ? <div className="loading-screen"><LoaderCircle size={22} className="spin" /><p>Opening your workspace…</p></div> : <div className="transcript">
        
        {projection.items.map(item => <Item key={item.id} item={item} onLink={onLink} />)}
        {hasPreview && <article className="message assistant-message streaming"><div className="message-heading"><span className="avatar agent-avatar"><Sparkles size={15} /></span><strong>Columbina</strong><span className="message-label">Writing</span></div>
          {preview.text ? <Markdown text={preview.text} onLink={onLink} /> : <div className="thinking"><LoaderCircle size={13} className="spin" />{preview.tools.length ? `Preparing ${preview.tools.map(tool => tool.name).join(', ')}…` : 'Thinking it through…'}</div>}
        </article>}
        {outputs.map(output => <details className="command-output" key={output.id} open><summary><Terminal size={13} /><code>{output.line}</code></summary><pre>{output.text}</pre></details>)}
        {queued.map((entry, index) => <div className="queued-message" key={`${entry.source}-${index}`}><Clock3 size={13} /><span>{messageText(entry.message)}</span><small>Queued</small></div>)}
        {projection.compacting && <div className="thinking"><LoaderCircle size={13} className="spin" />Making room in the conversation…</div>}
        {status?.status === 'running' && !hasPreview && !workspace.approval && !projection.compacting && <div className="working-indicator"><span /><span /><span /><small>Working</small></div>}
      </div>}
    </div>
    {away && <button className="jump-button" onClick={() => { if (scroll.current) scroll.current.scrollTo({ top: scroll.current.scrollHeight, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' }); stick.current = true; setAway(false); }}><ArrowDown size={14} />Latest message</button>}
  </div>;
}
