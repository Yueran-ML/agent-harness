import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { Markdown } from '../src/app/Markdown';

// What a model wrote is drawn as the page would draw it, and looked at as text.
const drawn = (text: string) => renderToStaticMarkup(<Markdown text={text} onLink={() => {}} />);

describe('what a model wrote', () => {
  it('is drawn as Markdown', () => {
    const page = drawn('# A title\n\nSome **strong** words and `code`.\n\n- one\n- two\n');
    expect(page).toContain('<h1>A title</h1>');
    expect(page).toContain('<strong>strong</strong>');
    expect(page).toContain('<code>code</code>');
    expect(page).toContain('<li>one</li>');
  });

  it('brings no HTML of its own into the page', () => {
    const page = drawn([
      '<script>alert(1)</script>',
      '<img src="https://example.com/x.png" onerror="alert(1)">',
      '<iframe src="https://example.com"></iframe>',
      '<a href="javascript:alert(1)">press</a>',
      '<style>body { display: none }</style>',
      'Words <b onclick="alert(1)">in</b> a line.',
    ].join('\n\n'));
    for (const tag of ['<script', '<img', '<iframe', '<a ', '<style', '<b ', 'onerror', 'onclick', 'javascript:']) {
      expect(page).not.toContain(tag);
    }
    // The words between the tags are still words.
    expect(page).toContain('a line.');
  });

  it('loads no picture, and says in words that one stood there', () => {
    const page = drawn('Look: ![a chart](https://example.com/chart.png) and ![](https://example.com/pixel.gif)');
    expect(page).not.toContain('<img');
    expect(page).not.toContain('example.com');
    expect(page).toContain('[a chart]');
  });

  it('makes of a link a button that goes nowhere by itself', () => {
    const page = drawn([
      'See [the documentation](https://example.com/docs).',
      'Or <https://example.com/plain>.',
      'Or https://example.com/bare in a line.',
      'Or [this](javascript:alert(1)) and [that](file:///C:/Windows/system.ini).',
    ].join('\n\n'));
    expect(page).not.toContain('<a');
    expect(page).not.toContain('href');
    expect(page).toContain('<button type="button" class="link">the documentation</button>');
    expect(page.match(/<button type="button" class="link">/g)?.length).toBeGreaterThanOrEqual(4);
  });

  it('shows code as it was written, tags and all', () => {
    const page = drawn('```html\n<script>alert("x")</script>\n```\n\nAnd `<b>inline</b>`.');
    expect(page).not.toContain('<script>');
    expect(page).toContain('&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;');
    expect(page).toContain('&lt;b&gt;inline&lt;/b&gt;');
  });

  it('keeps a wide table inside a part that scrolls', () => {
    const page = drawn('| a | b |\n|---|---|\n| 1 | 2 |\n');
    expect(page).toContain('<div class="table-wrap"><table>');
  });
});
