import { describe, expect, it } from 'vitest';
import type { CleanupActionKind, PlanGroupDto } from '../../api/types';
import { outcomeOf, previewOf, requestOf } from './preview';
import { GB, group, planOf, suggestion } from './testFixture';

const make = (id: string, action: CleanupActionKind, gb: number, extra: Partial<PlanGroupDto> = {}) => ({
  ...group(id, 'safe', gb),
  action,
  ...extra,
});

describe('outcomeOf', () => {
  it('sorts every action into what happens to the bytes', () => {
    expect(outcomeOf('trash')).toBe('trash');
    expect(outcomeOf('delete')).toBe('deleted');
    expect(outcomeOf('emptyFolder')).toBe('deleted');
    expect(outcomeOf('emptyTrash')).toBe('deleted');
    expect(outcomeOf('runTool')).toBe('tool');
  });
});

describe('previewOf', () => {
  const plan = planOf([
    make('small-tool', 'runTool', 3, { command: 'pacman -Sc', needsElevation: true }),
    make('big-delete', 'delete', 40),
    make('trash-a', 'trash', 5),
    make('empty', 'emptyTrash', 7),
    make('trash-b', 'trash', 9),
    make('folder', 'emptyFolder', 1),
  ]);
  const suggestions = [suggestion('trash-a', 'safe', 5), suggestion('big-delete', 'review', 40)];

  it('lists the rows by outcome, trash first, deleted second and tools last, the largest first inside each', () => {
    const rows = previewOf(plan, suggestions).rows;

    expect(rows.map((row) => row.ruleId)).toEqual([
      'trash-b',
      'trash-a',
      'big-delete',
      'empty',
      'folder',
      'small-tool',
    ]);
    expect(rows.map((row) => row.outcome)).toEqual([
      'trash',
      'trash',
      'deleted',
      'deleted',
      'deleted',
      'tool',
    ]);
  });

  it('adds up the bytes and the items of each outcome, and of all of them', () => {
    const preview = previewOf(plan, suggestions);

    expect(preview.totals).toEqual([
      { outcome: 'trash', bytes: 14 * GB, items: 2 },
      { outcome: 'deleted', bytes: 48 * GB, items: 3 },
      { outcome: 'tool', bytes: 3 * GB, items: 1 },
    ]);
    expect(preview.bytes).toBe(65 * GB);
    expect(preview.items).toBe(6);
  });

  it('keeps an outcome nothing goes to, with no bytes', () => {
    const preview = previewOf(planOf([make('only', 'trash', 2)]), []);

    expect(preview.totals.map((total) => total.bytes)).toEqual([2 * GB, 0, 0]);
  });

  it('words what is done, and keeps the command only for a tool', () => {
    const rows = previewOf(plan, suggestions).rows;
    const does = (id: string) => rows.find((row) => row.ruleId === id);

    expect(does('trash-a')?.does).toBe('Moved to the trash');
    expect(does('big-delete')?.does).toBe('Deleted');
    expect(does('empty')?.does).toBe('Emptied for good');
    expect(does('folder')?.does).toBe('Emptied');
    expect(does('small-tool')?.command).toBe('pacman -Sc');
    expect(does('small-tool')?.needsElevation).toBe(true);
    expect(does('trash-a')?.command).toBeUndefined();
    expect(does('trash-a')?.needsElevation).toBe(false);
  });

  it('takes the path and the reason from the suggestion with the same id, and leaves them out when none matches', () => {
    const rows = previewOf(plan, suggestions).rows;
    const known = rows.find((row) => row.ruleId === 'trash-a');
    const unknown = rows.find((row) => row.ruleId === 'empty');

    expect(known?.path).toBe('~/trash-a');
    expect(known?.reason).toBe('Reason of trash-a.');
    expect(unknown?.path).toBeUndefined();
    expect(unknown?.reason).toBeUndefined();
  });

  it('counts the refusals it lists and the ones it only counts', () => {
    const refused = { ruleId: 'x', path: '/p', reason: 'Kept.' };

    expect(previewOf(plan, suggestions).refused).toBe(0);
    expect(previewOf({ ...plan, refused: [refused, refused], refusedMore: 5 }, suggestions).refused).toBe(7);
  });
});

describe('requestOf', () => {
  const options = { keepPackageVersions: 2, askForAdmin: true } as const;
  const items = [{ id: 'a' }, { id: 'b' }];

  it('names the ids in order and picks only the two options of a plan', () => {
    const request = requestOf(items, { ...options, units: 'GB' } as typeof options);

    expect(request.ids).toEqual(['a', 'b']);
    expect(request.options).toEqual(options);
  });

  it('has the same key for the same request and another for another selection or option', () => {
    const key = requestOf(items, options).key;

    expect(requestOf([{ id: 'a' }, { id: 'b' }], { ...options }).key).toBe(key);
    expect(requestOf([{ id: 'a' }], options).key).not.toBe(key);
    expect(requestOf(items, { ...options, askForAdmin: false }).key).not.toBe(key);
    expect(requestOf(items, { ...options, keepPackageVersions: 3 }).key).not.toBe(key);
  });
});
