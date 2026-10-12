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

/** The red, green and blue of a colour that is written as `#rrggbb`. */
export function rgbOf(colour: string): [number, number, number] {
  return [1, 3, 5].map(at => Number.parseInt(colour.slice(at, at + 2), 16)) as [number, number, number];
}

/** A colour of a theme that lets through what is under it: its red, green and blue, and how much of it there is. */
export function washOf(theme: string, name: string): { rgb: [number, number, number]; alpha: number } {
  const found = new RegExp(`^\\s*--${name}:\\s*rgb\\((\\d+) (\\d+) (\\d+) / ([\\d.]+)\\);`, 'm')
    .exec(blockOf(tokens, selectorOf(theme)));
  if (!found) throw new Error(`The theme ${theme} gives --${name} no colour that lets something through.`);
  return { rgb: [Number(found[1]), Number(found[2]), Number(found[3])], alpha: Number(found[4]) };
}

/** A wash as it looks over a colour that lets nothing through. */
export function over(wash: { rgb: [number, number, number]; alpha: number }, under: [number, number, number]): [number, number, number] {
  return under.map((part, index) => part * (1 - wash.alpha) + wash.rgb[index] * wash.alpha) as [number, number, number];
}

// How light a colour is to the eye, between 0 and 1, as the guidelines for contrast define it.
function luminance(colour: [number, number, number]): number {
  const [red, green, blue] = colour.map((part) => {
    const share = part / 255;
    return share <= 0.03928 ? share / 12.92 : ((share + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

/** The contrast of two colours, from 1 for none to 21 for black on white. */
export function contrast(one: [number, number, number], other: [number, number, number]): number {
  const [dark, light] = [luminance(one), luminance(other)].sort((a, b) => a - b);
  return (light + 0.05) / (dark + 0.05);
}
