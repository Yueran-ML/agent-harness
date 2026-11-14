import type { UserConfig } from 'vite';
import { defineConfig, mergeConfig } from 'vitest/config';
import vite from './vite.config';

/**
 * Two projects. The performance tests (`*.perf.test.tsx`) time how long the page takes to draw, so
 * they run apart: after all the others are done (`groupOrder`), one file at a time in one process,
 * and not beside a crowd of other workers that would take the processor from them.
 */
export default mergeConfig(vite as UserConfig, defineConfig({
  test: {
    projects: [
      { extends: true, test: { name: 'unit', include: ['tests/**/*.test.{ts,tsx}'], exclude: ['tests/**/*.perf.test.{ts,tsx}'] } },
      {
        extends: true,
        test: {
          name: 'perf', include: ['tests/**/*.perf.test.{ts,tsx}'],
          pool: 'forks', poolOptions: { forks: { singleFork: true } }, sequence: { groupOrder: 1 },
        },
      },
    ],
  },
}));
