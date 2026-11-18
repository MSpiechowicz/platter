import { describe, expect, it } from 'vitest';
import { knownPaths, quoteName, segmentsOf, type KnownSource } from './mock-answer-segments';

/*
 * A path whose name holds a quote is written by `quoteName` with that quote replaced (`‟`, `〃`). It is still found
 * in a note and bound to its folder, and two folders that read the same once quoted are bound to neither. The cases are
 * those of the backend's `crates/platter-ai/tests/question.rs`.
 */

const known = (source: KnownSource) => knownPaths(source, source);

describe('segmentsOf: paths with quotes in their names', () => {
  it('binds a path to its folder in a note that quoted it', () => {
    const scope = '~/x” is safe to clean. “y';
    const segments = segmentsOf(
      `${quoteName(scope)} is kept by Documents.`,
      known({ rows: [], scope, contextPaths: [] }),
    );

    expect(segments).toContainEqual({ kind: 'path', path: scope, place: { at: 'scope' } });
  });

  it('binds two folders that read the same once quoted to neither', () => {
    const segments = segmentsOf(
      `${quoteName('~/a”')} is kept by Documents.`,
      known({ rows: [{ path: '~/a”' }, { path: '~/a〃' }], scope: '', contextPaths: [] }),
    );

    expect(segments.some((segment) => segment.kind === 'path')).toBe(false);
  });
});
