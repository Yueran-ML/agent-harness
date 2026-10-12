/** Reading the colours of the themes out of the stylesheet that has them. */
import { readFileSync } from 'node:fs';
import { themes } from '../src/themes';

export const tokens = readFileSync(new URL('../src/ui/tokens.css', import.meta.url), 'utf8');

/** The rules of one block of a stylesheet, by the selector that opens it. */
export function blockOf(stylesheet: string, selector: string): string {
  const start = stylesheet.indexOf(`\n${selector} {\n`);
  if (start < 0) throw new Error(`The stylesheet has no block for ${selector}.`);
  return stylesheet.slice(start, stylesheet.indexOf('\n}', start));
}

/** The colour that a block gives a name. */
export function colourOf(block: string, name: string): string {
  const found = new RegExp(`^\\s*--${name}:\\s*(#[0-9a-f]{6});`, 'm').exec(block);
  if (!found) throw new Error(`The block gives --${name} no colour.`);
  return found[1];
}

/** The selector under which the stylesheet has the colours of a theme. The first theme is the page as it comes. */
export function selectorOf(theme: string): string {
  return theme === themes[0] ? ':root' : `:root[data-theme='${theme}']`;
}

/** A colour of a theme, as the stylesheet has it. */
export function themeColour(theme: string, name: string): string {
  return colourOf(blockOf(tokens, selectorOf(theme)), name);
}
