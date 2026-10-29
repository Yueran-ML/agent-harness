import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { applyOp, type Op, type Project } from '../src/studio/engine';

interface Example { name: string; before: Project; after: Project; done: Op; inverse: Op }
const examples: Example[] = JSON.parse(readFileSync(new URL('../../engine/crates/model/tests/data/ops-examples.json', import.meta.url), 'utf8'));

describe('the protocol 2 project copy agrees with the Rust model', () => {
  for (const example of examples) {
    it(example.name, () => {
      const before = structuredClone(example.before);
      const changed = applyOp({ ...before, next_id: example.after.next_id }, example.done);
      expect(changed).toStrictEqual(example.after);
      expect(before).toStrictEqual(example.before);
      // Undo never reuses an id; compare at the next_id already allocated by the forward step.
      expect(applyOp(changed, example.inverse)).toStrictEqual({ ...before, next_id: example.after.next_id });
    });
  }
});
