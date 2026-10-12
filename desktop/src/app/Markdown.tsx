import { useState, type ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Check, Copy } from 'lucide-react';
import { useWords } from './locale';

/** A button that copies a text and says for a moment that it did. */
export function CopyButton({ text, className = 'icon-btn' }: { text: string; className?: string }) {
  const words = useWords();
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    }).catch(() => {
      // Without a clipboard the text can still be marked and copied by hand.
    });
  };
  const name = copied ? words.label.copied : words.label.copy;
  return <button type="button" className={className} onClick={copy} aria-label={name} title={name}>
    {copied ? <Check size={14} /> : <Copy size={14} />}
  </button>;
}

function textOf(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') return String(node);
  if (Array.isArray(node)) return node.map(textOf).join('');
  if (node !== null && typeof node === 'object' && 'props' in node) {
    return textOf((node as { props: { children?: ReactNode } }).props.children);
  }
  return '';
}

/**
 * What a model wrote, as Markdown.
 *
 * It is text that is not trusted: no HTML of it is used, no picture is
 * loaded, and a link does nothing by itself. It is handed to `onLink`, which
 * shows the address before anything opens.
 */
export function Markdown({ text, onLink }: { text: string; onLink: (url: string) => void }) {
  return <div className="markdown">
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      skipHtml
      components={{
        a: ({ href, children }) => <button type="button" className="link" onClick={() => href && onLink(href)}>
          {children}
        </button>,
        img: ({ alt }) => <span className="caption">[{alt || '…'}]</span>,
        pre: ({ children }) => <div className="code-block">
          <pre tabIndex={0}>{children}</pre>
          <CopyButton text={textOf(children).replace(/\n$/, '')} className="icon-btn code-copy" />
        </div>,
        table: ({ children }) => <div className="table-wrap"><table>{children}</table></div>,
      }}
    >{text}</ReactMarkdown>
  </div>;
}
