import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type Api } from '../../api/client';
import type { FolderMatchDto, UserRuleDto, UserRulesDto } from '../../api/types';
import { AdvisorStore } from '../../state/advisor.svelte';
import { RulesStore } from '../../state/rules.svelte';
import { SettingsStore } from '../../state/settings.svelte';
import { SEARCH_DELAY_MS } from './FolderPicker.svelte';
import RulesCard from './RulesCard.svelte';

const VIDEOS: UserRuleDto = { id: 1, path: '~/Videos', verdict: 'keep' };
const CACHE: UserRuleDto = { id: 2, path: '~/.cache/thumbnails', verdict: 'safe' };
const FOUND: FolderMatchDto = { node: 9, path: '/home/demo/Downloads', allocated: 96_000_000_000 };

interface Setup {
  list?: UserRulesDto;
  scan?: number | undefined;
  scanning?: boolean;
  api?: Partial<Api>;
}

async function setup(options: Setup = {}) {
  const api = {
    listUserRules: vi.fn().mockResolvedValue(options.list ?? { rules: [] }),
    addUserRule: vi.fn().mockResolvedValue({ rules: [VIDEOS] }),
    removeUserRule: vi.fn().mockResolvedValue({ rules: [] }),
    resetUserRules: vi.fn().mockResolvedValue({ rules: [] }),
    findFolders: vi.fn().mockResolvedValue([FOUND]),
    ...options.api,
  } as unknown as Api;
  const store = new AdvisorStore(api, new SettingsStore(null));
  const rulesChanged = vi.fn();
  const rules = new RulesStore(api, rulesChanged);

  await rules.load();

  const view = render(RulesCard, {
    props: { scan: 'scan' in options ? options.scan : 1, scanning: options.scanning, store, rules },
  });

  return { ...view, api, rules, rulesChanged };
}

const combobox = (): HTMLInputElement => screen.getByRole('combobox') as HTMLInputElement;

async function pick(text: string): Promise<void> {
  combobox().value = text;
  await fireEvent.input(combobox());
  await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
  await fireEvent.click(screen.getAllByRole('option')[0] as HTMLElement);
}

describe('RulesCard', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('lists the rules with the folder, KEEP or SAFE, and a way to remove each', async () => {
    await setup({ list: { rules: [VIDEOS, CACHE] } });

    const rows = [...document.querySelectorAll('.rule')];

    expect(rows.map((row) => row.querySelector('.path')?.textContent)).toEqual([
      '~/Videos',
      '~/.cache/thumbnails',
    ]);
    expect(rows.map((row) => row.querySelector('.tag')?.textContent)).toEqual(['keep', 'safe']);
    expect(screen.getByRole('button', { name: 'Remove the rule for ~/Videos' })).toBeTruthy();
    expect(screen.queryByText('No rules yet.')).toBeNull();
  });

  it('says in one line that there are no rules yet, and still offers the add form', async () => {
    await setup();

    expect(screen.getByText('No rules yet.')).toBeTruthy();
    expect(document.querySelectorAll('.rule')).toHaveLength(0);
    expect(combobox().disabled).toBe(false);
    expect(screen.getByRole('button', { name: 'Add' })).toBeTruthy();
  });

  it('shows a rule as text: a name with markup is not markup', async () => {
    await setup({ list: { rules: [{ id: 1, path: '~/<b>x</b>‮folder', verdict: 'keep' }] } });

    expect(document.querySelector('.path')?.textContent).toBe('~/<b>x</b>\\u{202E}folder');
    expect(document.querySelector('.path b')).toBeNull();
  });

  describe('when the rules file cannot be used', () => {
    const broken = { rules: [], problem: 'The rules file is not valid: expected value at line 1 column 1.' };

    it('shows the problem and Start over instead of the list and the add form', async () => {
      await setup({ list: broken });

      expect(screen.getByRole('alert').textContent).toContain(broken.problem);
      expect(screen.getByRole('button', { name: 'Start over' })).toBeTruthy();
      expect(screen.queryByRole('combobox')).toBeNull();
      expect(screen.queryByRole('button', { name: 'Add' })).toBeNull();
      expect(screen.queryByText('No rules yet.')).toBeNull();
    });

    it('starts over on a click: the file is reset and the empty list and the add form come back', async () => {
      const { api, rulesChanged } = await setup({ list: broken });

      await fireEvent.click(screen.getByRole('button', { name: 'Start over' }));
      await vi.advanceTimersByTimeAsync(0);

      expect(api.resetUserRules).toHaveBeenCalledTimes(1);
      expect(rulesChanged).toHaveBeenCalledTimes(1);
      expect(screen.queryByRole('button', { name: 'Start over' })).toBeNull();
      expect(screen.getByText('No rules yet.')).toBeTruthy();
      expect(combobox()).toBeTruthy();
    });

    it('says why starting over did not work, and keeps the button', async () => {
      await setup({
        list: broken,
        api: {
          resetUserRules: vi
            .fn()
            .mockRejectedValue(new ApiRequestError('io', 'The rules file could not be moved aside.')),
        },
      });

      await fireEvent.click(screen.getByRole('button', { name: 'Start over' }));
      await vi.advanceTimersByTimeAsync(0);

      expect(screen.getByText('The rules file could not be moved aside.')).toBeTruthy();
      expect(screen.getByRole('button', { name: 'Start over' })).toBeTruthy();
    });
  });

  describe('adding a rule', () => {
    it('adds a keep rule for the folder that was picked, and starts the field again', async () => {
      const { api, rulesChanged } = await setup();

      expect((screen.getByRole('button', { name: 'Add' }) as HTMLButtonElement).disabled).toBe(true);

      await pick('Down');

      expect(combobox().value).toBe('/home/demo/Downloads');

      await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
      await vi.advanceTimersByTimeAsync(0);

      expect(api.addUserRule).toHaveBeenCalledWith(1, 9, 'keep');
      expect(rulesChanged).toHaveBeenCalledTimes(1);
      expect(document.querySelector('.path')?.textContent).toBe('~/Videos');
      expect(combobox().value).toBe('');
      expect((screen.getByRole('button', { name: 'Add' }) as HTMLButtonElement).disabled).toBe(true);
    });

    it('adds on Enter in the field once a folder is picked', async () => {
      const { api } = await setup();

      await pick('Down');
      await fireEvent.keyDown(combobox(), { key: 'Enter' });
      await vi.advanceTimersByTimeAsync(0);

      expect(api.addUserRule).toHaveBeenCalledTimes(1);
    });

    it('shows a refusal as text under the form and keeps the folder picked', async () => {
      const refusal = 'The rules file lists more than 256 rules.';

      await setup({
        api: { addUserRule: vi.fn().mockRejectedValue(new ApiRequestError('invalid', refusal)) },
      });
      await pick('Down');
      await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
      await vi.advanceTimersByTimeAsync(0);

      expect(screen.getByRole('alert').textContent).toBe(refusal);
      expect(combobox().value).toBe('/home/demo/Downloads');
    });
  });

  describe('removing a rule', () => {
    it('removes the rule by its id and shows the list that came back', async () => {
      const { api } = await setup({ list: { rules: [VIDEOS] } });

      await fireEvent.click(screen.getByRole('button', { name: 'Remove the rule for ~/Videos' }));
      await vi.advanceTimersByTimeAsync(0);

      expect(api.removeUserRule).toHaveBeenCalledWith(1);
      expect(document.querySelectorAll('.rule')).toHaveLength(0);
      expect(screen.getByText('No rules yet.')).toBeTruthy();
    });

    it('says why a rule could not be removed, and keeps it', async () => {
      await setup({
        list: { rules: [VIDEOS] },
        api: {
          removeUserRule: vi
            .fn()
            .mockRejectedValue(new ApiRequestError('io', 'Your rules could not be saved: disk full')),
        },
      });

      await fireEvent.click(screen.getByRole('button', { name: 'Remove the rule for ~/Videos' }));
      await vi.advanceTimersByTimeAsync(0);

      expect(screen.getByRole('alert').textContent).toBe('Your rules could not be saved: disk full');
      expect(document.querySelectorAll('.rule')).toHaveLength(1);
    });
  });

  describe('without a finished scan', () => {
    it('still lists the rules, and keeps the add form off with the reason', async () => {
      await setup({ list: { rules: [VIDEOS] }, scan: undefined });

      expect(document.querySelector('.path')?.textContent).toBe('~/Videos');
      expect(combobox().disabled).toBe(true);
      expect((screen.getByRole('button', { name: 'Add' }) as HTMLButtonElement).disabled).toBe(true);
      expect(screen.getByText('Scan a disk or folder first.')).toBeTruthy();
    });

    it('says a scan is running while one is', async () => {
      await setup({ scan: undefined, scanning: true });

      expect(screen.getByText('A scan is running. The advisor uses it once it finishes.')).toBeTruthy();
    });

    it('can still remove a rule', async () => {
      const { api } = await setup({ list: { rules: [VIDEOS] }, scan: undefined });

      await fireEvent.click(screen.getByRole('button', { name: 'Remove the rule for ~/Videos' }));
      await vi.advanceTimersByTimeAsync(0);

      expect(api.removeUserRule).toHaveBeenCalledWith(1);
    });
  });
});
