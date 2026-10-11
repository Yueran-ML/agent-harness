import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Rollup } from 'vite';
import { beforeAll, describe, expect, it } from 'vitest';

const desktop = fileURLToPath(new URL('..', import.meta.url));

interface BuiltFile {
  name: string;
  text: string;
}

// The page is built as `npm run build` builds it, with the configuration of
// the project, but nothing is written: what would lie in `dist` is looked at
// in memory. So the test needs no earlier build and leaves nothing behind.
async function buildPage(): Promise<BuiltFile[]> {
  const result = await build({
    root: desktop,
    configFile: path.join(desktop, 'vite.config.ts'),
    logLevel: 'silent',
    build: { write: false },
  });
  if (!Array.isArray(result) && !('output' in result)) {
    throw new Error('The build watches for changes and gave no output.');
  }
  const outputs: Rollup.RollupOutput[] = Array.isArray(result) ? result : [result];
  return outputs.flatMap(output => output.output.map(file => ({
    name: file.fileName,
    text: file.type === 'chunk' ? file.code : textOf(file.source),
  })));
}

// A font file is not text. Latin-1 keeps every byte, so a search for ASCII
// in it finds what a search in the file on disk would find.
function textOf(source: string | Uint8Array): string {
  return typeof source === 'string' ? source : Buffer.from(source).toString('latin1');
}

// Every address that a stylesheet gives as the source of a font.
function fontAddresses(files: BuiltFile[]): { stylesheet: string; address: string }[] {
  const found = [];
  for (const stylesheet of files.filter(file => file.name.endsWith('.css'))) {
    for (const [face] of stylesheet.text.matchAll(/@font-face\s*\{[^}]*\}/g)) {
      for (const [, , address] of face.matchAll(/url\(\s*(['"]?)([^'")]+)\1\s*\)/g)) {
        found.push({ stylesheet: stylesheet.name, address });
      }
    }
  }
  return found;
}

describe('fonts in the built page', () => {
  let files: BuiltFile[] = [];
  // A build takes a few seconds, and longer on a machine that is busy.
  beforeAll(async () => { files = await buildPage(); }, 120_000);

  it('holds no font as a data address', () => {
    // Vite writes a small file into the stylesheet as a `data:` address unless
    // it is told not to, and the page refuses a font that comes that way.
    const inlined = files.filter(file => file.text.includes('data:font')).map(file => file.name);
    expect(inlined).toEqual([]);
  });

  it('takes every font from a file that the build wrote', () => {
    const written = new Set(files.map(file => file.name));
    const addresses = fontAddresses(files);
    expect(addresses.length).toBeGreaterThan(0);
    // An address is relative to the stylesheet that gives it.
    const elsewhere = addresses.filter(({ stylesheet, address }) =>
      !written.has(path.posix.join(path.posix.dirname(stylesheet), address)));
    // A `data:` address is as long as the font, so only its beginning is shown.
    expect(elsewhere.map(({ stylesheet, address }) => `${stylesheet}: ${address.slice(0, 60)}`)).toEqual([]);
  });
});

describe('the content security policy', () => {
  // The policy is stated twice: in the header that the shell sets for the
  // window, and in the page itself for the preview in a browser. Fonts are
  // written as files because of it, so it is held here as it is.
  it.each(['electron/main.cjs', 'index.html'])('lets %s load fonts from its own files only', (file) => {
    const source = readFileSync(path.join(desktop, file), 'utf8');
    const sources = [...source.matchAll(/font-src([^;"]*)/g)].map(match => match[1].trim());
    expect(sources).toEqual(["'self'"]);
  });
});
