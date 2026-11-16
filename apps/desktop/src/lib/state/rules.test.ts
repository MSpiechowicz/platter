import { describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type Api } from '../api/client';
import type { UserRulesDto } from '../api/types';
import { RulesStore } from './rules.svelte';

const keep = (id: number, path: string): UserRulesDto['rules'][number] => ({ id, path, verdict: 'keep' });

function setup(api: Partial<Api>) {
  const onChange = vi.fn();
  const store = new RulesStore(api as Api, onChange);

  return { store, onChange };
}

describe('RulesStore', () => {
  it('has read nothing at first, then shows the list as the backend gave it', async () => {
    const { store } = setup({
      listUserRules: vi.fn().mockResolvedValue({ rules: [keep(1, '~/Videos')] }),
    });

    expect(store.loaded).toBe(false);
    expect(store.rules).toEqual([]);

    await store.load();

    expect(store.loaded).toBe(true);
    expect(store.rules).toEqual([keep(1, '~/Videos')]);
    expect(store.broken).toBe(false);
  });

  it('reports why a list could not be read, and keeps what it had', async () => {
    const listUserRules = vi
      .fn()
      .mockResolvedValueOnce({ rules: [keep(1, '~/Videos')] })
      .mockRejectedValueOnce(new ApiRequestError('io', 'The settings folder cannot be read.'));
    const { store } = setup({ listUserRules });

    await store.load();
    await store.load();

    expect(store.loadFailure).toBe('The settings folder cannot be read.');
    expect(store.rules).toEqual([keep(1, '~/Videos')]);
  });

  it('shows the problem of a broken file, with no rules', async () => {
    const { store } = setup({
      listUserRules: vi.fn().mockResolvedValue({ rules: [], problem: 'The rules file is not valid.' }),
    });

    await store.load();

    expect(store.broken).toBe(true);
    expect(store.problem).toBe('The rules file is not valid.');
    expect(store.rules).toEqual([]);
  });

  it('adds a rule by scan, node and verdict, shows the new list and says the rules changed', async () => {
    const addUserRule = vi.fn().mockResolvedValue({ rules: [keep(1, '~/Videos')] });
    const { store, onChange } = setup({ addUserRule });

    expect(await store.add(3, 42, 'keep')).toBeUndefined();

    expect(addUserRule).toHaveBeenCalledWith(3, 42, 'keep');
    expect(store.rules).toEqual([keep(1, '~/Videos')]);
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it('returns the text of a refusal, changes nothing and says nothing changed', async () => {
    const { store, onChange } = setup({
      listUserRules: vi.fn().mockResolvedValue({ rules: [keep(1, '~/Videos')] }),
      addUserRule: vi
        .fn()
        .mockRejectedValue(
          new ApiRequestError('invalid', 'this is your home folder; it cannot be marked safe'),
        ),
    });

    await store.load();

    expect(await store.add(3, 42, 'safe')).toBe('this is your home folder; it cannot be marked safe');
    expect(store.rules).toEqual([keep(1, '~/Videos')]);
    expect(onChange).not.toHaveBeenCalled();
  });

  it('removes a rule by id and resets a broken file', async () => {
    const removeUserRule = vi.fn().mockResolvedValue({ rules: [] });
    const resetUserRules = vi.fn().mockResolvedValue({ rules: [] });
    const { store, onChange } = setup({
      listUserRules: vi.fn().mockResolvedValue({ rules: [], problem: 'Broken.' }),
      removeUserRule,
      resetUserRules,
    });

    await store.load();

    expect(store.broken).toBe(true);
    expect(await store.reset()).toBeUndefined();
    expect(store.broken).toBe(false);
    expect(await store.remove(5)).toBeUndefined();
    expect(removeUserRule).toHaveBeenCalledWith(5);
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('does not let a slow answer replace the list of a command that came after it', async () => {
    let slow: (list: UserRulesDto) => void = () => undefined;
    const addUserRule = vi
      .fn()
      .mockImplementationOnce(
        () =>
          new Promise<UserRulesDto>((resolve) => {
            slow = resolve;
          }),
      )
      .mockResolvedValueOnce({ rules: [keep(1, '~/a'), keep(2, '~/b')] });
    const { store } = setup({ addUserRule });

    const first = store.add(1, 1, 'keep');

    await store.add(1, 2, 'keep');
    slow({ rules: [keep(1, '~/a')] });
    await first;

    expect(store.rules.map((rule) => rule.path)).toEqual(['~/a', '~/b']);
  });
});
