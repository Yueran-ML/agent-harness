// @vitest-environment jsdom
/** Favourite Audio FX are listed again on their own, in the order they were starred (Reference Manual 20, p. 3697). */
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it } from 'vitest';
import { EffectsLibrary } from '../src/studio/fairlight/EffectsLibrary';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); localStorage.clear(); });
afterEach(() => { cleanup(); localStorage.clear(); });

it('lists the favourites apart, in the order starred, each still dragged as an effect', () => {
  render(<EffectsLibrary />);
  expect(screen.queryByRole('region', { name: 'Favorites' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Favorite Pitch' }));
  fireEvent.click(screen.getByRole('button', { name: 'Favorite Gain' }));
  const favorites = screen.getByRole('region', { name: 'Favorites' });
  expect([...favorites.querySelectorAll('.fairlight-effect span')].map(each => each.textContent)).toEqual(['Pitch', 'Gain']);
  // The library keeps its own order; a favourite is still dragged by its kind.
  expect(screen.getAllByText(/^(Gain|Limiter)$/)[0].textContent).toBe('Gain');
  const data = new Map<string, string>();
  fireEvent.dragStart(within(favorites).getByText('Pitch').parentElement!, { dataTransfer: { setData: (type: string, value: string) => data.set(type, value) } });
  expect([...data.values()]).toEqual(['pitch']);
  fireEvent.click(screen.getByRole('button', { name: 'Favorite Pitch' }));
  expect([...screen.getByRole('region', { name: 'Favorites' }).querySelectorAll('.fairlight-effect span')].map(each => each.textContent)).toEqual(['Gain']);
});
