import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../../api';
import type { AdvisorLabelsDto } from '../../api/types';
import { setMockAdvisorState, type MockAdvisorState } from '../../api/mock-advisor';
import { advisor } from '../../state/advisor.svelte';
import { rules } from '../../state/rules.svelte';
import { scan, type ReadyScan } from '../../state/scan.svelte';
import { LOADING_DELAY_MS, LOADING_MIN_VISIBLE_MS } from '../../state/delayed.svelte';
import { settings } from '../../state/settings.svelte';
import AdvisorPage from './AdvisorPage.svelte';

// The mock disk and the mock model, with no waiting for the scan.
vi.mock('../../api', async () => {
  const { createMockApi } = await import('../../api/mock');

  return { api: createMockApi({ durationMs: 0, now: () => 1_750_000_000 }) };
});

async function openWith(state: MockAdvisorState, withScan = false): Promise<void> {
  settings.set('advisorEnabled', true);
  await advisor.configure();
  setMockAdvisorState(state);
  await advisor.watch();

  let ready: ReadyScan | undefined;

  if (withScan) {
    await scan.start({ root: '/home/demo', oneFileSystem: true });
    await waitFor(() => expect(scan.state.status).toBe('ready'));
    ready = scan.state as ReadyScan;
  }

  render(AdvisorPage, { props: { ready } });

  // The page first reads its state behind one spinner, then shows the cards together.
  await waitFor(() => expect(document.querySelector('.waiting')).toBeNull());
}

const card = (name: string): HTMLElement => screen.getByRole('region', { name: new RegExp(`^${name}`) });

/** The state pill of the header. */
const pill = (): string => document.querySelector('.pill')?.textContent?.trim() ?? '';

describe('the first read of the Advisor view', () => {
  beforeEach(() => {
    scan.reset();
    advisor.labels = undefined;
  });

  it('shows one spinner in the middle until the state is read, then all the cards at once', async () => {
    settings.set('advisorEnabled', true);
    await advisor.configure();
    setMockAdvisorState('loaded');
    await advisor.watch();

    let finish: () => void = () => undefined;
    const read = vi.spyOn(rules, 'load').mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );

    render(AdvisorPage, { props: { ready: undefined } });

    // The rules are still being read: one waiting card, none of the cards yet.
    expect(document.querySelector('.waiting[aria-busy="true"]')).not.toBeNull();
    expect(screen.queryByRole('region', { name: /^Model/ })).toBeNull();
    expect(screen.queryByRole('region', { name: /^Ask about this scan/ })).toBeNull();

    finish();

    await waitFor(() => expect(document.querySelector('.waiting')).toBeNull());
    expect(card('Model')).toBeTruthy();
    expect(card('Ask about this scan')).toBeTruthy();
    expect(card('Needs your input')).toBeTruthy();

    read.mockRestore();
  });

  it('shows the cards without a status that never comes, after a few seconds', async () => {
    vi.useFakeTimers();

    try {
      advisor.status = undefined;
      render(AdvisorPage, { props: { ready: undefined } });

      await vi.advanceTimersByTimeAsync(1000);
      expect(document.querySelector('.waiting')).not.toBeNull();

      await vi.advanceTimersByTimeAsync(6000);
      expect(document.querySelector('.waiting')).toBeNull();
      expect(card('Ask about this scan')).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps the spinner a second once it appeared, even when the state comes just after', async () => {
    settings.set('advisorEnabled', true);
    await advisor.configure();
    setMockAdvisorState('loaded');
    await advisor.watch();

    let finish: () => void = () => undefined;
    const read = vi.spyOn(rules, 'load').mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );

    vi.useFakeTimers();

    try {
      render(AdvisorPage, { props: { ready: undefined } });

      // The read takes longer than 300 ms, so the spinner appears.
      await vi.advanceTimersByTimeAsync(LOADING_DELAY_MS + 10);
      expect(screen.getByRole('status')).toBeTruthy();

      // The state comes in just after: the spinner stays out its second, with no card yet.
      finish();
      await vi.advanceTimersByTimeAsync(LOADING_MIN_VISIBLE_MS - 200);
      expect(document.querySelector('.waiting')).not.toBeNull();
      expect(screen.queryByRole('region', { name: /^Model/ })).toBeNull();

      await vi.advanceTimersByTimeAsync(400);
      expect(document.querySelector('.waiting')).toBeNull();
      expect(card('Model')).toBeTruthy();
    } finally {
      vi.useRealTimers();
      read.mockRestore();
    }
  });
});

describe('the Advisor view without a scan', () => {
  beforeEach(() => {
    scan.reset();
    advisor.exchanges = [];
    advisor.labels = undefined;
    advisor.failure = undefined;
    advisor.labelFailure = undefined;
    settings.set('units', 'GB');
    settings.set('advisorUseGpu', true);
  });

  it('shows a loaded model: pill, Unload button, real values and the video memory it holds', async () => {
    await openWith('loaded');

    expect(screen.getByText('ADVISOR')).toBeTruthy();
    expect(pill()).toBe('Model loaded');
    expect(screen.getByRole('button', { name: 'Unload model' })).toBeTruthy();

    const model = card('Model');

    expect(within(model).getByText('Qwen3.5-4B')).toBeTruthy();
    expect(within(model).getByText('GGUF · Q4_K_M')).toBeTruthy();
    expect(within(model).getByText('2.7 GB')).toBeTruthy();
    expect(within(model).getByText('5 min idle')).toBeTruthy();
    expect(within(model).getByText('GPU · AMD Radeon RX 7900 XTX')).toBeTruthy();
    expect(within(model).getByText('3.2 GB video memory in use')).toBeTruthy();
    expect(within(model).queryByRole('meter')).toBeNull();
    expect(within(model).getByText("The model download is Platter's only network call.")).toBeTruthy();
  });

  it('says what the model can and cannot see', async () => {
    await openWith('loaded');

    for (const text of [
      'Folder and file names',
      'Sizes, counts and dates',
      'Which rules matched nearby',
      'File contents',
      'The network (except the one-time model download)',
    ]) {
      expect(screen.getByText(text)).toBeTruthy();
    }
  });

  it('says "Scan a disk or folder first." in every card that is about a scan', async () => {
    await openWith('loaded');

    expect(within(card('How a verdict is made')).getByText('Scan a disk or folder first.')).toBeTruthy();
    expect(within(card('Ask about this scan')).getByText('Scan a disk or folder first.')).toBeTruthy();
    expect(within(card('Needs your input')).getByText('Scan a disk or folder first.')).toBeTruthy();
    expect(within(card('What to keep')).getByText('Scan a disk or folder first.')).toBeTruthy();
    expect(within(card('Ask about this scan')).getByRole('combobox')).toHaveProperty('disabled', true);
    expect(within(card('What to keep')).getByRole('combobox')).toHaveProperty('disabled', true);
  });

  it('puts What to keep in the right column, under Needs your input, and reads the rules when it opens', async () => {
    await openWith('loaded');

    const needs = card('Needs your input');
    const yours = card('What to keep');

    expect(needs.parentElement).toBe(yours.parentElement);
    expect(needs.nextElementSibling).toBe(yours);
    expect(await within(yours).findByText('No rules yet.')).toBeTruthy();
  });

  it('welcomes a model that is not there with one download button, and shows its progress with Cancel', async () => {
    await openWith('none');

    expect(pill()).toBe('Not downloaded');
    expect(screen.getByRole('heading', { name: 'Download the local model to use the Advisor' })).toBeTruthy();
    expect(screen.queryByRole('region', { name: /^Model/ })).toBeNull();
    // One primary action on the page: the header has none.
    expect(screen.queryByRole('button', { name: 'Download (2.7 GB)' })).toBeNull();

    await fireEvent.click(screen.getByRole('button', { name: 'Download Qwen3.5-4B (2.7 GB)' }));

    await waitFor(() => expect(pill()).toBe('Downloading…'));
    await waitFor(() => expect(screen.getByRole('progressbar')).toBeTruthy());
    expect(screen.queryByRole('button', { name: 'Cancel download' })).toBeNull();

    await waitFor(() => expect(screen.getByText(/(MB|GB) of 2\.7 GB · \d+%/)).toBeTruthy());
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    await waitFor(() => expect(pill()).toBe('Not downloaded'));
    expect(screen.getByRole('button', { name: /^Continue download/ })).toBeTruthy();
  });

  it('shows the welcome, not the cards, for a download in progress, with no header button', async () => {
    await openWith('downloading');

    expect(pill()).toBe('Downloading…');
    expect(screen.getByText('1.1 GB of 2.7 GB · 40%')).toBeTruthy();
    expect(screen.queryByRole('region', { name: /^Model/ })).toBeNull();
    expect(screen.queryByRole('region', { name: /^Ask about this scan/ })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Cancel download' })).toBeNull();
    expect(screen.getAllByRole('button', { name: 'Cancel' })).toHaveLength(1);
  });

  it('shows the cards again once the model is on disk', async () => {
    await openWith('none');
    expect(screen.queryByRole('region', { name: /^Model/ })).toBeNull();

    setMockAdvisorState('disk');

    expect(await screen.findByRole('region', { name: /^Model/ })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: 'Download the local model to use the Advisor' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Load model' })).toBeTruthy();
  });

  it('loads a model that is on disk when the button is pressed', async () => {
    await openWith('disk');

    expect(pill()).toBe('Model on disk, not loaded');
    expect(within(card('Model')).queryByRole('meter')).toBeNull();

    await fireEvent.click(screen.getByRole('button', { name: 'Load model' }));

    await waitFor(() => expect(pill()).toBe('Model loaded'));
    expect(within(card('Model')).getByText('3.2 GB video memory in use')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Unload model' })).toBeTruthy();
  });

  it('shows the processor and the RAM it holds when the graphics card is switched off', async () => {
    settings.set('advisorUseGpu', false);
    await openWith('loaded');

    const model = card('Model');

    expect(within(model).getByText('CPU')).toBeTruthy();
    expect(within(model).getByText('2.1 GB of 34 GB RAM in use')).toBeTruthy();
    expect(within(model).getByRole('meter')).toBeTruthy();
    expect(within(model).queryByText(/video memory/)).toBeNull();
  });

  it('does not offer to load a model that is switched off in the settings, and says so', async () => {
    await openWith('disk');
    await advisor.setEnabled(false);

    await waitFor(() => {
      const button = screen.getByRole('button', { name: 'Load model' }) as HTMLButtonElement;

      expect(button.disabled).toBe(true);
      expect(button.title).toBe('The local model is switched off in Settings.');
    });
    expect(within(card('Model')).getByText('The local model is switched off in Settings.')).toBeTruthy();
  });
});

describe('the Advisor view with a scan', () => {
  beforeEach(() => {
    scan.reset();
    advisor.exchanges = [];
    advisor.labels = undefined;
    advisor.failure = undefined;
    advisor.labelFailure = undefined;
    settings.set('units', 'GB');
  });

  it('counts the folders of each step before any labelling, and invites to label', async () => {
    await openWith('loaded', true);

    const verdict = card('How a verdict is made');

    await within(verdict).findByText('Built-in rules');
    expect(within(verdict).getByText('Local model')).toBeTruthy();
    expect(within(verdict).getByText('Safety guard')).toBeTruthy();
    expect(within(verdict).queryByText(/model time/)).toBeNull();

    const needs = card('Needs your input');

    expect(await within(needs).findByRole('button', { name: 'Label folders' })).toBeTruthy();
    expect((within(needs).getByRole('button', { name: 'Label folders' }) as HTMLButtonElement).disabled).toBe(
      false,
    );
    // The actions on a folder belong to a later package: no dead buttons.
    expect(screen.queryByRole('button', { name: 'Always keep' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Safe to clean' })).toBeNull();
  });

  it('labels the folders with progress and Cancel, then lists what needs a look with its reason', async () => {
    await openWith('loaded', true);

    const needs = card('Needs your input');

    await fireEvent.click(await within(needs).findByRole('button', { name: 'Label folders' }));

    await waitFor(() =>
      expect(within(needs).getByText(/^Labelling folders: \d+ of \d+ folders$/)).toBeTruthy(),
    );
    expect(within(needs).getByRole('button', { name: 'Cancel' })).toBeTruthy();

    await waitFor(() => expect(within(needs).queryByText(/^Labelling folders/)).toBeNull(), {
      timeout: 8000,
    });

    await waitFor(() => expect(needs.querySelectorAll('.item').length).toBeGreaterThan(0));
    expect(needs.querySelector('.hint')?.textContent).toMatch(/^\d+ folders?$/);
    expect(needs.querySelector('.item .path')?.textContent).toMatch(/^\//);
    expect(needs.querySelector('.item small')?.textContent?.length).toBeGreaterThan(5);
    // The model time and the split of the verdict card come from the same labels.
    await waitFor(() =>
      expect(within(card('How a verdict is made')).getByText(/^model time \d/)).toBeTruthy(),
    );
  }, 15_000);

  it('stops a labelling at Cancel and says it stopped early', async () => {
    await openWith('loaded', true);

    const needs = card('Needs your input');

    await fireEvent.click(await within(needs).findByRole('button', { name: 'Label folders' }));
    await waitFor(() => within(needs).getByRole('button', { name: 'Cancel' }));
    await new Promise((resolve) => setTimeout(resolve, 700));
    await fireEvent.click(within(needs).getByRole('button', { name: 'Cancel' }));

    await waitFor(() =>
      expect(within(needs).getByText('Stopped before every folder was labelled.')).toBeTruthy(),
    );
    expect(within(needs).getByRole('button', { name: 'Label again' })).toBeTruthy();
  });

  it('answers a typed question about the scan with rows, and asks about one of the folders from its row', async () => {
    await openWith('loaded', true);

    const ask = card('Ask about this scan');
    const input = within(ask).getByRole('combobox') as HTMLInputElement;

    // No example chips: nothing to press before the user types.
    expect(within(ask).queryByRole('button', { name: /^Ask about / })).toBeNull();
    expect((within(ask).getByRole('button', { name: 'Ask' }) as HTMLButtonElement).disabled).toBe(true);

    await fireEvent.input(input, { target: { value: 'what are the biggest folders?' } });
    await fireEvent.click(within(ask).getByRole('button', { name: 'Ask' }));

    expect(within(ask).getByText('what are the biggest folders?')).toBeTruthy();
    expect(input.disabled).toBe(true);
    // The model takes a moment to read this one, so the wait is shown once it has lasted.
    expect((await within(ask).findByRole('status')).textContent).toContain('Reading your question…');

    await waitFor(() => expect(ask.querySelectorAll('.row').length).toBeGreaterThan(0), { timeout: 5000 });
    expect(within(ask).queryByRole('status')).toBeNull();
    expect(ask.querySelector('.row .tag')).toBeTruthy();
    expect(ask.querySelector('.row .size')?.textContent).toMatch(/\d/);

    await fireEvent.click(within(ask).getAllByRole('button', { name: /^Ask about / })[0] as HTMLElement);

    expect(await within(ask).findByText(/^Can I delete /)).toBeTruthy();
    await waitFor(() => expect(ask.querySelector('.msg.a .v')).toBeTruthy(), { timeout: 5000 });
    expect(ask.querySelector('.msg.a .v')?.textContent).toMatch(
      /Safety guard|Built-in rule|No rule|Local model/,
    );
    // The rules answer at once: no wait, and nothing about a model's wording.
    expect(within(ask).queryByRole('status')).toBeNull();
    expect(ask.querySelector('.msg.a .v')?.textContent).not.toContain('Answered by the local model');
  }, 15_000);

  it('answers off-topic text with a fixed line and no rows', async () => {
    await openWith('loaded', true);

    const ask = card('Ask about this scan');

    await fireEvent.input(within(ask).getByRole('combobox'), { target: { value: 'write me a poem' } });
    await fireEvent.click(within(ask).getByRole('button', { name: 'Ask' }));

    expect(
      await within(ask).findByText(/^I can only answer questions about this scan/, {}, { timeout: 5000 }),
    ).toBeTruthy();
    expect(ask.querySelector('.rows')).toBeNull();
  }, 15_000);
});

describe('the Advisor view while a scan runs', () => {
  beforeEach(() => {
    scan.reset();
    advisor.exchanges = [];
    advisor.labels = undefined;
    advisor.failure = undefined;
    advisor.labelFailure = undefined;
  });

  it('says the scan is running, not that there is none, in every card that is about a scan', async () => {
    settings.set('advisorEnabled', true);
    await advisor.configure();
    setMockAdvisorState('loaded');
    await advisor.watch();

    render(AdvisorPage, { props: { ready: undefined, scanning: true } });
    await waitFor(() => expect(document.querySelector('.waiting')).toBeNull());

    const text = 'A scan is running. The advisor uses it once it finishes.';

    for (const name of ['How a verdict is made', 'Ask about this scan', 'Needs your input', 'What to keep']) {
      expect(within(card(name)).getByText(text)).toBeTruthy();
      expect(within(card(name)).queryByText('Scan a disk or folder first.')).toBeNull();
    }
  });
});

describe('the labels of the scan while several requests overlap', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('keeps "reading" until the latest request has answered, and never shows a missing answer for a dropped one', async () => {
    const held: { resolve: (value: AdvisorLabelsDto) => void; reject: (error: Error) => void }[] = [];
    const real = api.getAdvisorLabels.bind(api);

    settings.set('advisorEnabled', true);
    await advisor.configure();
    setMockAdvisorState('loaded');
    await advisor.watch();
    await scan.start({ root: '/home/demo', oneFileSystem: true });
    await waitFor(() => expect(scan.state.status).toBe('ready'));

    const ready = scan.state as ReadyScan;
    const labels = await real(ready.scanId);

    advisor.labels = undefined;
    vi.spyOn(api, 'getAdvisorLabels').mockImplementation(
      () =>
        new Promise<AdvisorLabelsDto>((resolve, reject) => {
          held.push({ resolve, reject });
        }),
    );

    render(AdvisorPage, { props: { ready } });
    await waitFor(() => expect(held).toHaveLength(1));

    // A refetch (a labelling that ended, a cleanup) replaces the request of the page.
    const newer = advisor.loadLabels(ready.scanId);

    held[0]?.resolve(labels);
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(screen.queryByText('The counts could not be read.')).toBeNull();
    expect(screen.queryByText('The labels could not be read.')).toBeNull();
    expect(within(card('How a verdict is made')).queryByText('Built-in rules')).toBeNull();

    held[1]?.reject(new Error('unreadable'));
    await newer;

    expect(await screen.findByText('The counts could not be read.')).toBeTruthy();
    expect(screen.getByText('The labels could not be read.')).toBeTruthy();
  });
});

describe('the user rules on the Advisor view', () => {
  beforeEach(() => {
    scan.reset();
    advisor.exchanges = [];
    advisor.labels = undefined;
  });

  afterEach(async () => {
    // The mock keeps its rules for the whole file.
    for (const rule of (await api.listUserRules()).rules) {
      await api.removeUserRule(rule.id);
    }

    await rules.load();
  });

  it('adds a rule for a picked folder, shows it, and the advisor then answers about that folder with it', async () => {
    await openWith('loaded', true);

    const yours = card('What to keep');
    const input = within(yours).getByRole('combobox') as HTMLInputElement;
    const versionBefore = advisor.suggestionsVersion;

    input.value = 'Downloads';
    await fireEvent.input(input);
    await within(yours).findByRole('option');
    await fireEvent.keyDown(input, { key: 'Enter' });
    await fireEvent.click(within(yours).getByRole('button', { name: 'Add' }));

    expect(await within(yours).findByText('~/Downloads')).toBeTruthy();
    expect(yours.querySelector('.rule .tag')?.textContent).toBe('keep');
    expect(advisor.suggestionsVersion).toBeGreaterThan(versionBefore);

    const ask = card('Ask about this scan');

    await fireEvent.input(within(ask).getByRole('combobox'), { target: { value: 'where is Downloads?' } });
    await fireEvent.click(within(ask).getByRole('button', { name: 'Ask' }));
    await fireEvent.click(
      await within(ask).findByRole('button', { name: /^Ask about .*Downloads$/ }, { timeout: 5000 }),
    );

    await waitFor(() => expect(ask.querySelector('.msg.a .v')).toBeTruthy(), { timeout: 5000 });
    expect(ask.querySelector('.msg.a .v')?.textContent).toMatch(
      /Your rule\s*Your rule: Folders you keep\. [\d.]+ [KMGT]?B\./,
    );
  }, 15_000);

  it('removes a rule again', async () => {
    await openWith('loaded', true);

    const folder = (await api.findFolders((scan.state as ReadyScan).scanId, '/home/demo/Downloads', 1))[0];

    await api.addUserRule((scan.state as ReadyScan).scanId, folder?.node as number, 'keep');
    await rules.load();

    const yours = card('What to keep');

    await fireEvent.click(within(yours).getByRole('button', { name: 'Remove the rule for ~/Downloads' }));

    expect(await within(yours).findByText('No rules yet.')).toBeTruthy();
  });
});
