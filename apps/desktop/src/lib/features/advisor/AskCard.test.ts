import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type Api } from '../../api/client';
import type { AdvisorAnswerDto, EntryMatchDto, ScanAnswerDto } from '../../api/types';
import { AdvisorStore, FIND_LIMIT } from '../../state/advisor.svelte';
import { LOADING_DELAY_MS } from '../../state/delayed.svelte';
import { SettingsStore } from '../../state/settings.svelte';
import { ui } from '../../state/ui.svelte';
import AskCard from './AskCard.svelte';
import { SEARCH_DELAY_MS } from './FolderPicker.svelte';
import { scanAnswer, scanRow, statusOf } from './fixtures';

/** The name of the chip that `scanAnswer()` has in its text: the folder and Platter's verdict on it. */
const VIDEOS_CHIP = '/home/demo/Videos, kept';

const EMPTY_LINE =
  'Ask anything about what was scanned. The local model only reads your question. Platter answers from the scan and its rules.';

/** What Platter says about one folder: its own words, from a rule. */
const folderAnswer = (over: Partial<AdvisorAnswerDto> = {}): AdvisorAnswerDto => ({
  node: 11,
  path: '/home/demo/Videos',
  verdict: 'keep',
  reason: 'Your own videos. Left alone on purpose. Nothing inside is suggested for cleanup.',
  source: 'rule',
  detail: 'Rule: Videos. 12 GB. 5% of it (600 MB) is older than a year.',
  ...over,
});

function setup(
  askScan: Api['askScan'],
  status = statusOf('loaded'),
  findEntries: Api['findEntries'] = vi.fn().mockResolvedValue([]),
) {
  const askAdvisor = vi.fn().mockResolvedValue(folderAnswer());
  const settings = new SettingsStore(null);
  const store = new AdvisorStore({ askScan, askAdvisor, findEntries } as unknown as Api, settings);

  settings.set('advisorEnabled', true);
  store.status = status;

  const view = render(AskCard, { props: { scan: 1, store } });

  return { ...view, store, settings, askAdvisor, findEntries };
}

const input = (): HTMLInputElement => screen.getByRole('combobox') as HTMLInputElement;
const send = (): HTMLButtonElement => screen.getByRole('button', { name: 'Ask' }) as HTMLButtonElement;

async function ask(text: string): Promise<void> {
  await fireEvent.input(input(), { target: { value: text } });
  await fireEvent.click(send());
}

describe('AskCard: asking a free question', () => {
  beforeEach(() => {
    ui.go('advisor');
  });

  it('is a plain question box: the owner’s empty line, the placeholder, and no example chips', () => {
    setup(vi.fn());

    expect(screen.getByRole('region', { name: 'Ask about this scan' })).toBeTruthy();
    expect(screen.getByText(EMPTY_LINE)).toBeTruthy();
    expect(input().placeholder).toBe('Ask about this scan, in your own words');
    expect(screen.queryAllByRole('button').map((button) => button.textContent?.trim())).toEqual([
      'Ask',
      'Clear',
    ]);
    expect(send().disabled).toBe(true);
  });

  it('sends the typed question with the units of the settings and shows the answer text and its rows', async () => {
    const askScan = vi.fn().mockResolvedValue(scanAnswer());
    const { settings } = setup(askScan);

    settings.set('units', 'GB');
    await ask('  what takes the most space?  ');

    expect(askScan).toHaveBeenCalledWith(1, 'what takes the most space?', 'GB');
    expect(await screen.findByRole('button', { name: VIDEOS_CHIP })).toBeTruthy();
    expect(screen.getByText('what takes the most space?')).toBeTruthy();
    expect(input().value).toBe('');

    const rows = screen.getAllByRole('listitem').filter((item) => item.classList.contains('row'));

    expect(rows).toHaveLength(2);
    expect(within(rows[0] as HTMLElement).getByText('/home/demo/Videos')).toBeTruthy();
    expect(within(rows[0] as HTMLElement).getByText('12 GB')).toBeTruthy();
    expect(within(rows[0] as HTMLElement).getByText('keep')).toBeTruthy();
    expect(within(rows[0] as HTMLElement).getByText('Safety guard')).toBeTruthy();
    expect(within(rows[0] as HTMLElement).getByText('Your own files')).toBeTruthy();
    expect(within(rows[1] as HTMLElement).getByText('1.3 GB')).toBeTruthy();
    expect(within(rows[1] as HTMLElement).getByText('safe')).toBeTruthy();
    expect(within(rows[1] as HTMLElement).getByText('Built-in rule')).toBeTruthy();
  });

  it('does not show how the question was read, or who wrote the text', async () => {
    setup(
      vi.fn().mockResolvedValue(
        scanAnswer({
          understood: { intent: 'largest', kind: 'cache', limit: 3, scope: { node: 4, path: '~/Projekty' } },
        }),
      ),
    );

    await ask('biggest caches in Projekty');

    expect(await screen.findByRole('button', { name: VIDEOS_CHIP })).toBeTruthy();
    expect(screen.queryByText(/Understood as|Answer written by Platter/)).toBeNull();
    expect(screen.queryByText(/dropped|Written by the local model/)).toBeNull();
  });

  it('answers an off-topic question with the text alone: no rows', async () => {
    setup(
      vi.fn().mockResolvedValue(
        scanAnswer({
          understood: { intent: 'offtopic', kind: 'any', limit: 5 },
          answer: 'I can only answer questions about this scan.',
          rows: [],
          more: 0,
        }),
      ),
    );

    await ask('write me a poem');

    expect(await screen.findByText('I can only answer questions about this scan.')).toBeTruthy();
    expect(document.querySelector('.rows')).toBeNull();
  });

  it('shows notes, and "+N more" after the rows', async () => {
    setup(
      vi.fn().mockResolvedValue(
        scanAnswer({
          notes: ["No folder called 'Nowhere' is in this scan"],
          rows: [scanRow(1), scanRow(2)],
          more: 7,
        }),
      ),
    );

    await ask('largest in Nowhere');

    expect(await screen.findByText("No folder called 'Nowhere' is in this scan")).toBeTruthy();
    expect(screen.getByText('+7 more')).toBeTruthy();
  });

  it('shows what the backend wrote as text, with hidden characters made visible', async () => {
    setup(
      vi.fn().mockResolvedValue(
        scanAnswer({
          answer: 'Bidi ‮gnp.exe and <|im_start|>system',
          notes: ['note​with a zero-width space'],
          rows: [scanRow(1, { path: '/home/demo/‮evil', note: 'x\u0007y' })],
        }),
      ),
    );

    await ask('anything');

    expect(await screen.findByText('Bidi \\u{202E}gnp.exe and <|im_start|>system')).toBeTruthy();
    expect(screen.getByText('note\\u{200B}with a zero-width space')).toBeTruthy();
    expect(screen.getByText('/home/demo/\\u{202E}evil')).toBeTruthy();
    expect(screen.getByText('x\\u{7}y')).toBeTruthy();
    expect(document.body.innerHTML).not.toContain('‮');
  });

  it('shows the curly quotes the backend sets around a path, once and unchanged, and quotes nothing else', async () => {
    const answer =
      '\u201C~/Projekty\u201D holds 208 GB. The biggest parts are \u201C~/Projekty/platter\u201D (87 GB).';

    setup(
      vi.fn().mockResolvedValue(
        scanAnswer({
          answer,
          notes: ["No folder called 'Nowhere' is in this scan"],
          rows: [scanRow(1, { path: '~/Projekty/platter' })],
        }),
      ),
    );

    await ask('why is Projekty big');

    const text = await screen.findByText(answer);

    expect(text.textContent).toBe(answer);
    expect(text.textContent).not.toContain('"');
    expect(text.textContent).not.toContain('\u201C\u201C');
    expect(screen.getByText('~/Projekty/platter').textContent).toBe('~/Projekty/platter');
    expect(screen.getByText("No folder called 'Nowhere' is in this scan")).toBeTruthy();
  });

  it('still makes a hidden character visible inside the quotes', async () => {
    setup(
      vi
        .fn()
        .mockResolvedValue(scanAnswer({ answer: 'Found \u201C~/a\u202Eb\u201D (1 GB).', rows: [], more: 0 })),
    );

    await ask('where is b');

    expect(await screen.findByText('Found \u201C~/a\\u{202E}b\u201D (1 GB).')).toBeTruthy();
  });

  it('cuts a long path at the end, and keeps the full path in the tooltip and the name of the link', async () => {
    const long = `/home/demo/${'deeply-nested-folder/'.repeat(8)}end`;

    setup(vi.fn().mockResolvedValue(scanAnswer({ rows: [scanRow(1, { path: long })] })));
    await ask('largest');

    const link = await screen.findByRole('button', { name: `Open ${long} in the Explorer` });

    expect(link.title).toContain(long);
    expect(link.querySelector('.p')?.textContent).toBe(long);
  });

  it('opens a row’s folder in the Explorer when its path is clicked', async () => {
    setup(vi.fn().mockResolvedValue(scanAnswer()));
    await ask('largest');

    await fireEvent.click(
      await screen.findByRole('button', { name: 'Open /home/demo/.cache in the Explorer' }),
    );

    expect(ui.page).toBe('explorer');
    expect(ui.explorerFocus).toBe(12);
  });

  it('runs the folder question from the "Ask" of a row with the units of the settings, and the verdict joins the log', async () => {
    const { askAdvisor, settings } = setup(vi.fn().mockResolvedValue(scanAnswer()));

    settings.set('units', 'GiB');
    await ask('largest');
    await fireEvent.click(await screen.findByRole('button', { name: 'Ask about /home/demo/Videos' }));

    expect(askAdvisor).toHaveBeenCalledWith(1, 11, 'GiB');
    expect(await screen.findByText('Can I delete /home/demo/Videos?')).toBeTruthy();

    const reply = (await screen.findByText(/Your own videos\. Left alone on purpose/)).closest('.bd');

    expect(within(reply as HTMLElement).getByText('Built-in rule')).toBeTruthy();
    expect(
      within(reply as HTMLElement).getByText('Rule: Videos. 12 GB. 5% of it (600 MB) is older than a year.'),
    ).toBeTruthy();
  });

  it('shows Platter’s own words and the name of who decided for each kind of answer, and never a word of the model’s wording', async () => {
    const cases: [Partial<AdvisorAnswerDto>, string][] = [
      [{ source: 'user', verdict: 'keep', detail: 'Your rule: Folders you keep. 1 GB.' }, 'Your rule'],
      [
        {
          source: 'guard',
          verdict: 'keep',
          reason:
            'Left alone by Platter. This is a protected place, or inside one (/etc). Platter does not clean it.',
          detail: '1 GB.',
        },
        'Safety guard',
      ],
      [
        {
          source: 'unruled',
          verdict: 'review',
          reason: 'Platter has no rule for it. Deleting it is your call.',
          detail: '315 MB.',
        },
        'No rule',
      ],
      [
        {
          source: 'model',
          verdict: 'review',
          reason:
            'Platter has no rule for it. Deleting it is your call. The local model marked it for review.',
          detail: '315 MB.',
        },
        'Local model',
      ],
    ];

    for (const [over, source] of cases) {
      const { askAdvisor, unmount } = setup(vi.fn().mockResolvedValue(scanAnswer()));

      askAdvisor.mockResolvedValue(folderAnswer(over));

      await ask('largest');
      await fireEvent.click(await screen.findByRole('button', { name: 'Ask about /home/demo/Videos' }));

      const reply = (await screen.findByText(over.reason ?? /Your own videos/)).closest('.bd') as HTMLElement;

      expect(within(reply).getByText(source), source).toBeTruthy();
      expect(within(reply).getByText(over.verdict as string)).toBeTruthy();
      expect(reply.textContent).not.toContain('Answered by the local model');
      expect(reply.textContent).not.toContain('Thinking');

      unmount();
    }
  });

  it('shows no wait for a folder question: nothing like "Thinking…" shows however long it takes', async () => {
    const { askAdvisor } = setup(vi.fn().mockResolvedValue(scanAnswer()));

    askAdvisor.mockImplementation(() => new Promise<AdvisorAnswerDto>(() => undefined));

    await ask('largest');
    await fireEvent.click(await screen.findByRole('button', { name: 'Ask about /home/demo/Videos' }));

    expect(await screen.findByText('Can I delete /home/demo/Videos?')).toBeTruthy();

    // Longer than the card waits before it shows the wait of a free question.
    await new Promise((resolve) => setTimeout(resolve, LOADING_DELAY_MS + 100));

    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.queryByText(/Thinking|Reading your question/)).toBeNull();
    expect(screen.getAllByText('Advisor')).toHaveLength(1);
  });
});

describe('AskCard: while the answer is on its way, and when it fails', () => {
  it('says what is happening and keeps the question box shut until the answer comes', async () => {
    let answer: (value: ScanAnswerDto) => void = () => undefined;
    const askScan = vi.fn(
      () =>
        new Promise<ScanAnswerDto>((resolve) => {
          answer = resolve;
        }),
    );

    setup(askScan);
    await ask('what takes the most space?');

    expect(input().disabled).toBe(true);
    expect(send().disabled).toBe(true);
    expect((await screen.findByRole('status')).textContent).toContain('Reading your question…');

    answer(scanAnswer());

    expect(await screen.findByRole('button', { name: VIDEOS_CHIP })).toBeTruthy();
    expect(screen.queryByRole('status')).toBeNull();
    await waitFor(() => expect(input().disabled).toBe(false));
  });

  it('shows no wait for an answer that comes back at once, and the wait only once it has lasted a moment', async () => {
    let answer: (value: ScanAnswerDto) => void = () => undefined;
    const askScan = vi
      .fn<Api['askScan']>()
      .mockResolvedValueOnce(scanAnswer())
      .mockImplementationOnce(
        () =>
          new Promise<ScanAnswerDto>((resolve) => {
            answer = resolve;
          }),
      );

    setup(askScan);

    // Every moment the wait is on screen is counted, so a flash that comes and goes would show.
    let shown = 0;
    const observer = new MutationObserver(() => {
      if (document.querySelector('[role="status"]')) {
        shown++;
      }
    });

    observer.observe(document.body, { childList: true, subtree: true });

    await ask('what takes the most space?');
    expect(await screen.findByRole('button', { name: VIDEOS_CHIP })).toBeTruthy();
    expect(screen.getAllByText('Advisor')).toHaveLength(1);
    await new Promise((resolve) => setTimeout(resolve, 400));
    observer.disconnect();

    expect(shown).toBe(0);

    // A slow one: nothing beside the question at first, then the wait.
    await ask('biggest folders');

    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.getAllByText('Advisor')).toHaveLength(1);
    expect(await screen.findByRole('status')).toBeTruthy();
    expect(screen.getAllByText('Advisor')).toHaveLength(2);

    answer(scanAnswer({ answer: 'Done.' }));

    expect(await screen.findByText('Done.')).toBeTruthy();
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('keeps the question box and every row Ask shut while a question about another scan is on its way', async () => {
    let answer: (value: ScanAnswerDto) => void = () => undefined;
    const askScan = vi.fn(
      () =>
        new Promise<ScanAnswerDto>((resolve) => {
          answer = resolve;
        }),
    );
    const { store } = setup(askScan);

    // The question about scan 2 is still on its way; the card shows scan 1, which has none of its own.
    void store.askScan(2, 'what takes the most space?');

    await waitFor(() => expect(input().disabled).toBe(true));
    expect(screen.queryByRole('status')).toBeNull();
    expect(send().disabled).toBe(true);
    expect(screen.getByText('A question about another scan is still being answered.')).toBeTruthy();

    answer(scanAnswer({ scan: 2 }));
    await waitFor(() => expect(input().disabled).toBe(false));
    expect(screen.queryByText('A question about another scan is still being answered.')).toBeNull();
    expect(askScan).toHaveBeenCalledTimes(1);
  });

  it('shuts Try again of a free question while another is on its way, but never the "Ask" of a row', async () => {
    let answer: (value: ScanAnswerDto) => void = () => undefined;
    const askScan = vi
      .fn<Api['askScan']>()
      .mockResolvedValueOnce(scanAnswer())
      .mockRejectedValueOnce(new ApiRequestError('invalid', 'The model is busy.'))
      .mockImplementationOnce(
        () =>
          new Promise<ScanAnswerDto>((resolve) => {
            answer = resolve;
          }),
      );
    const { store, askAdvisor } = setup(askScan);

    await ask('largest');
    await ask('biggest folders');

    const rowAsk = (): HTMLButtonElement =>
      screen.getByRole('button', { name: 'Ask about /home/demo/Videos' }) as HTMLButtonElement;
    const retry = (): HTMLButtonElement =>
      screen.getByRole('button', { name: 'Try again' }) as HTMLButtonElement;

    await screen.findByRole('alert');
    expect(retry().disabled).toBe(false);
    expect(rowAsk().disabled).toBe(false);

    // A question of another scan starts: the failed one cannot be sent again, but a row can still be asked about.
    void store.askScan(2, 'overview');

    await waitFor(() => expect(retry().disabled).toBe(true));
    expect(rowAsk().disabled).toBe(false);

    await fireEvent.click(retry());
    await fireEvent.click(rowAsk());

    expect(askScan).toHaveBeenCalledTimes(3);
    expect(askAdvisor).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(/Your own videos\. Left alone on purpose/)).toBeTruthy();

    answer(scanAnswer({ scan: 2 }));

    await waitFor(() => expect(retry().disabled).toBe(false));
    expect(rowAsk().disabled).toBe(false);
  });

  it('keeps the "Ask" of a row open whatever the model is doing: not downloaded, switched off, labelling or loading', async () => {
    const states = [
      statusOf('notDownloaded'),
      statusOf('downloading'),
      statusOf('onDisk'),
      statusOf('loading'),
      statusOf('unavailable'),
      statusOf('loaded', { labelling: { scan: 1, done: 2, total: 9 } }),
    ];

    for (const status of states) {
      const { store, settings, askAdvisor, unmount } = setup(vi.fn().mockResolvedValue(scanAnswer()), status);

      // The box is shut for the model's states; a free question is put to the store without it.
      await store.askScan(1, 'largest');

      settings.set('advisorEnabled', false);

      const rowAsk = (await screen.findByRole('button', {
        name: 'Ask about /home/demo/Videos',
      })) as HTMLButtonElement;

      expect(rowAsk.disabled, status.state).toBe(false);

      await fireEvent.click(rowAsk);

      expect(askAdvisor, status.state).toHaveBeenCalledTimes(1);
      expect(await screen.findByText(/Your own videos\. Left alone on purpose/)).toBeTruthy();

      unmount();
    }
  });

  it('shows the reason as text with Try again, and asks the same question again', async () => {
    const askScan = vi
      .fn()
      .mockRejectedValueOnce(new ApiRequestError('invalid', 'The model is not downloaded.'))
      .mockResolvedValueOnce(scanAnswer({ answer: 'Second time works.' }));

    setup(askScan);
    await ask('biggest folders');

    expect((await screen.findByRole('alert')).textContent).toContain('The model is not downloaded.');
    expect(input().disabled).toBe(false);

    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    expect(await screen.findByText('Second time works.')).toBeTruthy();
    expect(askScan).toHaveBeenCalledTimes(2);
    expect(askScan.mock.calls[1]).toEqual(askScan.mock.calls[0]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('keeps the question box shut with the reason while the model is not downloaded', () => {
    setup(vi.fn(), statusOf('notDownloaded'));

    expect(input().disabled).toBe(true);
    expect(send().disabled).toBe(true);
    expect(screen.getByText('Download the model first.')).toBeTruthy();
  });

  it('keeps the question box shut while the model labels folders', () => {
    setup(vi.fn(), statusOf('loaded', { labelling: { scan: 1, done: 2, total: 9 } }));

    expect(input().disabled).toBe(true);
    expect(screen.getByText(/labelling folders/)).toBeTruthy();
  });

  it('says there is no scan to ask about, and shuts the box', () => {
    const store = new AdvisorStore({} as Api, new SettingsStore(null));

    render(AskCard, { props: { scan: undefined, store } });

    expect(screen.getByText('Scan a disk or folder first.')).toBeTruthy();
    expect(input().disabled).toBe(true);
  });
});

describe('AskCard: a question asked back', () => {
  const QUESTION = 'can I delete the backup folder?';
  const asked = (): ScanAnswerDto =>
    scanAnswer({
      understood: { intent: 'cleanable', kind: 'any', limit: 5, scopeText: 'backup' },
      answer: 'I found more than one folder called \u201Cbackup\u201D. Which one do you mean?',
      segments: [
        {
          kind: 'text',
          text: 'I found more than one folder called \u201Cbackup\u201D. Which one do you mean?',
        },
      ],
      rows: [],
      clarify: {
        prompt: 'I found more than one folder called \u201Cbackup\u201D. Which one do you mean?',
        choices: [
          { id: 0, label: '~/work/backup', path: '~/work/backup', node: 21 },
          { id: 1, label: '~/phone/backup', path: '~/phone/backup', node: 22 },
        ],
      },
    });
  const picked = (): ScanAnswerDto =>
    scanAnswer({
      understood: {
        intent: 'cleanable',
        kind: 'any',
        limit: 5,
        scope: { node: 22, path: '~/phone/backup' },
      },
      answer: 'The picked answer.',
      segments: [{ kind: 'text', text: 'The picked answer.' }],
      rows: [],
    });

  it('shows the prompt and one button per choice, and nothing that would answer the question', async () => {
    setup(vi.fn().mockResolvedValue(asked()));
    await ask(QUESTION);

    expect(
      await screen.findByText(
        'I found more than one folder called \u201Cbackup\u201D. Which one do you mean?',
      ),
    ).toBeTruthy();

    const choices = within(screen.getByRole('list', { name: 'Which one do you mean?' })).getAllByRole(
      'button',
    ) as HTMLButtonElement[];

    expect(choices.map((choice) => choice.textContent?.trim())).toEqual(['~/work/backup', '~/phone/backup']);
    expect(choices.map((choice) => choice.title)).toEqual([
      '~/work/backup: ask about this one',
      '~/phone/backup: ask about this one',
    ]);
    // The same folder glyph a row's path has, then the path.
    expect(choices.every((choice) => choice.querySelector('svg') && choice.querySelector('.cpath'))).toBe(
      true,
    );
    expect(choices.every((choice) => !choice.disabled)).toBe(true);
    expect(document.querySelector('.rows, .notes, .how')).toBeNull();
    expect(screen.queryByText(/Understood as/)).toBeNull();
  });

  it('asks the same question again with the id of the choice, as a new exchange that says what was picked', async () => {
    const askScan = vi.fn<Api['askScan']>().mockResolvedValueOnce(asked()).mockResolvedValueOnce(picked());
    const { settings } = setup(askScan);

    settings.set('units', 'GB');
    await ask(QUESTION);
    await fireEvent.click(await screen.findByRole('button', { name: '~/phone/backup' }));

    expect(await screen.findByText('The picked answer.')).toBeTruthy();
    // The same text and the choice's id: never a path.
    expect(askScan.mock.calls).toEqual([
      [1, QUESTION, 'GB'],
      [1, QUESTION, 'GB', 1],
    ]);
    expect(screen.getByText('You picked: ~/phone/backup')).toBeTruthy();
    // The question and its prompt stay above the pick.
    expect(screen.getByText(QUESTION)).toBeTruthy();
    expect(document.querySelectorAll('.msg.q')).toHaveLength(2);
  });

  it('shuts the choices while a question is on its way, and opens them when it ends', async () => {
    let answer: (value: ScanAnswerDto) => void = () => undefined;
    const askScan = vi
      .fn<Api['askScan']>()
      .mockResolvedValueOnce(asked())
      .mockImplementationOnce(
        () =>
          new Promise<ScanAnswerDto>((resolve) => {
            answer = resolve;
          }),
      );

    setup(askScan);
    await ask(QUESTION);
    await fireEvent.click(await screen.findByRole('button', { name: '~/work/backup' }));

    const choice = (name: string): HTMLButtonElement =>
      screen.getByRole('button', { name }) as HTMLButtonElement;

    await waitFor(() => expect(choice('~/phone/backup').disabled).toBe(true));
    expect(choice('~/work/backup').disabled).toBe(true);

    // A second click while the first is on its way asks nothing.
    await fireEvent.click(choice('~/phone/backup'));
    expect(askScan).toHaveBeenCalledTimes(2);

    answer(picked());

    await waitFor(() => expect(choice('~/phone/backup').disabled).toBe(false));
    expect(choice('~/work/backup').disabled).toBe(false);
  });

  it('shuts the choices while the model cannot be asked', async () => {
    const { store } = setup(vi.fn().mockResolvedValue(asked()));

    await ask(QUESTION);
    await screen.findByRole('button', { name: '~/work/backup' });

    store.status = statusOf('loaded', { labelling: { scan: 1, done: 2, total: 9 } });

    await waitFor(() =>
      expect((screen.getByRole('button', { name: '~/work/backup' }) as HTMLButtonElement).disabled).toBe(
        true,
      ),
    );
  });

  it('shows a choice that is no folder (a tool) by its label, and a long path cut at the end with the whole one in the tooltip', async () => {
    const long = `~/${'deeply-nested-folder/'.repeat(8)}backup`;
    const answer = asked();

    answer.clarify = {
      prompt: answer.clarify?.prompt ?? '',
      choices: [
        { id: 0, label: 'node_modules' },
        { id: 1, label: long, path: long, node: 7 },
      ],
    };
    setup(vi.fn().mockResolvedValue(answer));
    await ask(QUESTION);

    const tool = await screen.findByRole('button', { name: 'node_modules' });
    const folder = screen.getByRole('button', { name: long });

    expect(tool.querySelector('svg')).toBeNull();
    expect(tool.title).toBe('');
    expect(folder.title).toBe(`${long}: ask about this one`);
    expect(folder.querySelector('.cpath')?.textContent).toBe(long);
  });

  it('shows a refused pick as the reason with Try again, like any question that failed', async () => {
    const askScan = vi
      .fn<Api['askScan']>()
      .mockResolvedValueOnce(asked())
      .mockRejectedValueOnce(
        new ApiRequestError('invalid', 'that choice is no longer available, so ask again'),
      );

    setup(askScan);
    await ask(QUESTION);
    await fireEvent.click(await screen.findByRole('button', { name: '~/work/backup' }));

    expect((await screen.findByRole('alert')).textContent).toContain(
      'that choice is no longer available, so ask again',
    );
    expect(screen.getByText('You picked: ~/work/backup')).toBeTruthy();
  });
});

describe('AskCard: clearing the conversation', () => {
  const clearButton = (): HTMLButtonElement =>
    screen.getByRole('button', { name: 'Clear' }) as HTMLButtonElement;

  it('puts Clear after Ask at the bottom, shut while there is nothing to clear', () => {
    setup(vi.fn());

    const buttons = [...document.querySelectorAll('form.inp button')].map((button) =>
      button.textContent?.trim(),
    );

    expect(buttons).toEqual(['Ask', 'Clear']);
    expect(clearButton().disabled).toBe(true);
  });

  it('takes the exchanges of this scan off the screen, keeps those of another scan, and refocuses the box', async () => {
    const { store } = setup(vi.fn().mockResolvedValue(scanAnswer()));

    await store.askScan(2, 'overview');
    await ask('largest');
    await ask('biggest folders');
    await screen.findAllByRole('button', { name: VIDEOS_CHIP });
    await waitFor(() => expect(input().disabled).toBe(false));

    await fireEvent.click(clearButton());

    expect(screen.getByText(EMPTY_LINE)).toBeTruthy();
    expect(screen.queryByText('largest')).toBeNull();
    expect(store.exchangesOf(1)).toEqual([]);
    expect(store.exchangesOf(2)).toHaveLength(1);
    expect(clearButton().disabled).toBe(true);
    await waitFor(() => expect(document.activeElement).toBe(input()));
  });

  it('is shut while a question is on its way, and clears nothing then', async () => {
    let answer: (value: ScanAnswerDto) => void = () => undefined;
    const { store } = setup(
      vi.fn(
        () =>
          new Promise<ScanAnswerDto>((resolve) => {
            answer = resolve;
          }),
      ),
    );

    await ask('largest');

    expect(clearButton().disabled).toBe(true);

    store.clearExchanges(1);

    expect(store.exchangesOf(1)).toHaveLength(1);

    answer(scanAnswer());
    await screen.findByRole('button', { name: VIDEOS_CHIP });
    await waitFor(() => expect(clearButton().disabled).toBe(false));
  });

  it('is shut while a question about another scan is on its way', async () => {
    let answer: (value: ScanAnswerDto) => void = () => undefined;
    const askScan = vi
      .fn<Api['askScan']>()
      .mockResolvedValueOnce(scanAnswer())
      .mockImplementationOnce(
        () =>
          new Promise<ScanAnswerDto>((resolve) => {
            answer = resolve;
          }),
      );
    const { store } = setup(askScan);

    await ask('largest');
    await screen.findByRole('button', { name: VIDEOS_CHIP });

    void store.askScan(2, 'overview');

    await waitFor(() => expect(clearButton().disabled).toBe(true));

    answer(scanAnswer({ scan: 2 }));

    await waitFor(() => expect(clearButton().disabled).toBe(false));
  });
});

describe('AskCard: the log scrolls and the question box stays', () => {
  const HEIGHT = 1000;
  const VIEW = 400;
  const QUESTION_TOP = 900;

  /** The log as a browser would measure it: a conversation of HEIGHT in a view of VIEW, scrolled to `top`. */
  function measure(log: HTMLElement): { top: () => number; scrollTo: (top: number) => void } {
    let top = 0;

    Object.defineProperty(log, 'scrollHeight', { configurable: true, get: () => HEIGHT });
    Object.defineProperty(log, 'clientHeight', { configurable: true, get: () => VIEW });
    Object.defineProperty(log, 'scrollTop', {
      configurable: true,
      get: () => top,
      set: (value: number) => {
        top = value;
      },
    });

    return { top: () => top, scrollTo: (value) => (top = value) };
  }

  /** A question whose answer comes when the test says so. */
  function held() {
    let answer: (value: ScanAnswerDto) => void = () => undefined;
    const askScan = vi.fn(
      () =>
        new Promise<ScanAnswerDto>((resolve) => {
          answer = resolve;
        }),
    );

    return { askScan, answer: (value: ScanAnswerDto) => answer(value) };
  }

  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, 'offsetTop', 'get').mockImplementation(function (this: HTMLElement) {
      return this.classList.contains('q') ? QUESTION_TOP : 0;
    });
  });

  it('keeps the question box and its button outside the log, so they do not scroll away with it', async () => {
    setup(vi.fn().mockResolvedValue(scanAnswer()));
    await ask('largest');
    await screen.findByRole('button', { name: VIDEOS_CHIP });

    const log = screen.getByRole('log');

    expect(log.textContent).toContain('largest');
    expect(log.contains(input())).toBe(false);
    expect(log.contains(send())).toBe(false);
    expect(log.closest('section')?.contains(input())).toBe(true);
  });

  it('shows a question that was sent at once', async () => {
    const pending = held();

    setup(pending.askScan);

    const scroll = measure(screen.getByRole('log'));

    await ask('largest');

    await waitFor(() => expect(scroll.top()).toBe(HEIGHT - VIEW));
  });

  it('follows the answer when the reader is at the end, and brings its question into view', async () => {
    const pending = held();

    setup(pending.askScan);

    const log = screen.getByRole('log');
    const scroll = measure(log);

    await ask('largest');
    await waitFor(() => expect(scroll.top()).toBe(HEIGHT - VIEW));

    // 40px from the end is still reading the end; a wheel turn does not change that.
    scroll.scrollTo(HEIGHT - VIEW - 40);
    await fireEvent.wheel(log);
    pending.answer(scanAnswer());
    await screen.findByRole('button', { name: VIDEOS_CHIP });

    await waitFor(() => expect(scroll.top()).toBe(HEIGHT - VIEW));
  });

  it('does not take the reader away from older answers when an answer arrives', async () => {
    const pending = held();

    setup(pending.askScan);

    const log = screen.getByRole('log');
    const scroll = measure(log);

    await ask('largest');
    await waitFor(() => expect(scroll.top()).toBe(HEIGHT - VIEW));

    scroll.scrollTo(100);
    await fireEvent.wheel(log);
    await fireEvent.scroll(log);

    pending.answer(scanAnswer());
    await screen.findByRole('button', { name: VIDEOS_CHIP });
    await new Promise((resolve) => setTimeout(resolve, 50));

    expect(scroll.top()).toBe(100);
  });

  it('shows the question the reader sends even when they were reading older answers', async () => {
    const answers = vi.fn().mockResolvedValue(scanAnswer());

    setup(answers);

    const log = screen.getByRole('log');
    const scroll = measure(log);

    await ask('largest');
    await screen.findByRole('button', { name: VIDEOS_CHIP });
    await waitFor(() => expect(input().disabled).toBe(false));

    scroll.scrollTo(100);
    await fireEvent.wheel(log);
    await ask('biggest folders');

    await waitFor(() => expect(scroll.top()).toBe(HEIGHT - VIEW));
  });

  it('starts a long answer in view: from its question when the two do not fit', async () => {
    const pending = held();

    setup(pending.askScan);

    const log = screen.getByRole('log');
    const scroll = measure(log);

    await ask('largest');
    await waitFor(() => expect(scroll.top()).toBe(HEIGHT - VIEW));

    // The conversation is longer now: the end of the log is below the question's place.
    Object.defineProperty(log, 'scrollHeight', { configurable: true, get: () => 2000 });
    pending.answer(scanAnswer());
    await screen.findByRole('button', { name: VIDEOS_CHIP });

    await waitFor(() => expect(scroll.top()).toBe(QUESTION_TOP));
  });
});

const DOWNLOADS: EntryMatchDto = { node: 4, path: '~/Pobrane', allocated: 96_000_000_000, kind: 'dir' };
const DOCUMENTS: EntryMatchDto = { node: 5, path: '~/Dokumenty', allocated: 4_000_000_000, kind: 'dir' };
const OLD_ISO: EntryMatchDto = { node: 6, path: '~/Pobrane/old.iso', allocated: 4_200_000_000, kind: 'file' };

/** Types at the end of the field, as the keys do, and waits out the pause after the last key. */
async function typeIn(text: string, settle = true): Promise<void> {
  input().value = text;
  await fireEvent.input(input());

  if (settle) {
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
  }
}

const options = (): HTMLElement[] => screen.queryAllByRole('option');
const active = (): string | undefined =>
  options()
    .find((option) => option.getAttribute('aria-selected') === 'true')
    ?.querySelector('.p')?.textContent ?? undefined;

describe('AskCard: completing the word under the caret', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    ui.go('advisor');
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('asks 150 ms after the last key, once, for the word typed', async () => {
    const { findEntries } = setup(vi.fn(), statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS]));

    await typeIn('~/Po', false);
    await vi.advanceTimersByTimeAsync(100);
    await typeIn('~/Pob', false);
    await vi.advanceTimersByTimeAsync(100);

    expect(findEntries).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(50);

    expect(findEntries).toHaveBeenCalledTimes(1);
    expect(findEntries).toHaveBeenCalledWith(1, '~/Pob', FIND_LIMIT);
  });

  it('makes no call for a word of 2 characters', async () => {
    const { findEntries } = setup(vi.fn(), statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS]));

    await typeIn('ab');
    await typeIn('what is ab?');

    expect(findEntries).not.toHaveBeenCalled();
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('lists each match above the field with its path (a slash after a folder), its size and what it is', async () => {
    setup(vi.fn(), statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS, OLD_ISO]));

    await typeIn('~/Pob');

    expect(input().getAttribute('aria-expanded')).toBe('true');
    expect(input().getAttribute('aria-controls')).toBe(screen.getByRole('listbox').id);
    expect(input().getAttribute('aria-autocomplete')).toBe('list');
    expect(options().map((option) => option.getAttribute('aria-label'))).toEqual([
      'Folder ~/Pobrane/, 96 GB',
      'File ~/Pobrane/old.iso, 4.2 GB',
    ]);
    expect(options()[0]?.textContent).toContain('~/Pobrane/');
    expect(options()[0]?.textContent).toContain('96 GB');
    expect(options()[1]?.textContent).toContain('~/Pobrane/old.iso');
    expect(input().getAttribute('aria-activedescendant')).toBe(options()[0]?.id);
  });

  it('moves the active option with the arrow keys and stops at both ends', async () => {
    setup(vi.fn(), statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS, DOCUMENTS]));

    await typeIn('~/');

    expect(active()).toBe('~/Pobrane/');

    await fireEvent.keyDown(input(), { key: 'ArrowDown' });
    expect(active()).toBe('~/Dokumenty/');
    expect(input().getAttribute('aria-activedescendant')).toBe(options()[1]?.id);

    await fireEvent.keyDown(input(), { key: 'ArrowDown' });
    expect(active()).toBe('~/Dokumenty/');

    await fireEvent.keyDown(input(), { key: 'ArrowUp' });
    await fireEvent.keyDown(input(), { key: 'ArrowUp' });
    expect(active()).toBe('~/Pobrane/');
  });

  it('accepts the active option with Tab, keeps the caret after the slash and asks for the next level', async () => {
    const findEntries = vi.fn().mockResolvedValueOnce([DOWNLOADS, DOCUMENTS]).mockResolvedValue([OLD_ISO]);

    setup(vi.fn(), statusOf('loaded'), findEntries);

    await typeIn('~/Pob');

    const kept = await fireEvent.keyDown(input(), { key: 'Tab' });

    expect(kept).toBe(false);
    expect(input().value).toBe('~/Pobrane/');
    expect(input().selectionStart).toBe(10);

    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);

    expect(findEntries).toHaveBeenLastCalledWith(1, '~/Pobrane/', FIND_LIMIT);
    expect(options()).toHaveLength(1);
    expect(options()[0]?.textContent).toContain('~/Pobrane/old.iso');
  });

  it('takes a file with a click, adds a space and closes the list', async () => {
    const { findEntries } = setup(vi.fn(), statusOf('loaded'), vi.fn().mockResolvedValue([OLD_ISO]));

    await typeIn('is old.iso big');
    await typeIn('is old.iso');
    await fireEvent.click(screen.getByRole('option'));
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);

    expect(input().value).toBe('is ~/Pobrane/old.iso ');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(findEntries).toHaveBeenCalledTimes(2);
  });

  it('takes Enter with the list open as a pick, not as the question', async () => {
    const askScan = vi.fn().mockResolvedValue(scanAnswer());

    setup(askScan, statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS]));

    await typeIn('size of ~/Pob');

    const submitted = await fireEvent.keyDown(input(), { key: 'Enter' });

    expect(submitted).toBe(false);
    expect(input().value).toBe('size of ~/Pobrane/');
    expect(askScan).not.toHaveBeenCalled();
  });

  it('closes the list with Escape and leaves the text', async () => {
    setup(vi.fn(), statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS]));

    await typeIn('~/Pob');
    expect(screen.getByRole('listbox')).toBeTruthy();

    await fireEvent.keyDown(input(), { key: 'Escape' });

    expect(screen.queryByRole('listbox')).toBeNull();
    expect(input().getAttribute('aria-expanded')).toBe('false');
    expect(input().value).toBe('~/Pob');
  });

  it('leaves the arrow keys to the caret while the list is closed', async () => {
    setup(vi.fn(), statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS, DOCUMENTS]));

    await typeIn('size of ~/Pob');
    await fireEvent.keyDown(input(), { key: 'Escape' });

    expect(await fireEvent.keyDown(input(), { key: 'ArrowDown' })).toBe(true);
    expect(await fireEvent.keyDown(input(), { key: 'ArrowUp' })).toBe(true);
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(input().getAttribute('aria-expanded')).toBe('false');
  });

  it('drops the search on its way when the caret goes to another word, so its result never opens', async () => {
    let answer: (found: EntryMatchDto[]) => void = () => undefined;
    const findEntries = vi.fn(
      () =>
        new Promise<EntryMatchDto[]>((resolve) => {
          answer = resolve;
        }),
    );

    setup(vi.fn(), statusOf('loaded'), findEntries);

    // The caret is in `~/Pob`, and the search for it is on its way.
    input().value = 'size of ~/Pob big?';
    input().setSelectionRange(13, 13);
    await fireEvent.input(input());
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);

    expect(findEntries).toHaveBeenCalledTimes(1);

    // A key within the same word keeps it.
    input().setSelectionRange(12, 12);
    await fireEvent.keyUp(input(), { key: 'ArrowLeft' });

    // A click on another word does not.
    input().setSelectionRange(17, 17);
    await fireEvent.click(input());
    answer([DOWNLOADS]);
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.queryByRole('listbox')).toBeNull();
    expect(input().getAttribute('aria-expanded')).toBe('false');
  });

  it('does not offer a name the question would read as another entry', async () => {
    const crafted: EntryMatchDto = { node: 9, path: '~/Pobrane "draft" v2.txt', allocated: 1, kind: 'file' };
    const findEntries = vi.fn(async () => [crafted, DOWNLOADS]);

    setup(vi.fn(), statusOf('loaded'), findEntries);

    input().value = 'size of ~/Pob';
    input().setSelectionRange(13, 13);
    await fireEvent.input(input());
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);

    expect(options()).toHaveLength(1);
    expect(options()[0].textContent).toContain('~/Pobrane/');
  });

  it('drops a result when the caret left its word by a key no handler sees', async () => {
    let answer: (found: EntryMatchDto[]) => void = () => undefined;
    const findEntries = vi.fn(
      () =>
        new Promise<EntryMatchDto[]>((resolve) => {
          answer = resolve;
        }),
    );

    setup(vi.fn(), statusOf('loaded'), findEntries);

    input().value = 'size of ~/Pob big?';
    input().setSelectionRange(13, 13);
    await fireEvent.input(input());
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);

    expect(findEntries).toHaveBeenCalledTimes(1);

    // ArrowUp in a one-line field puts the caret at the start, with no event the card listens to.
    input().setSelectionRange(0, 0);
    answer([DOWNLOADS]);
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.queryByRole('listbox')).toBeNull();
    expect(input().getAttribute('aria-expanded')).toBe('false');
  });

  it('shows the result of the word the caret stayed in', async () => {
    let answer: (found: EntryMatchDto[]) => void = () => undefined;
    const findEntries = vi.fn(
      () =>
        new Promise<EntryMatchDto[]>((resolve) => {
          answer = resolve;
        }),
    );

    setup(vi.fn(), statusOf('loaded'), findEntries);

    input().value = 'size of ~/Pob big?';
    input().setSelectionRange(13, 13);
    await fireEvent.input(input());
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);

    input().setSelectionRange(11, 11);
    await fireEvent.keyUp(input(), { key: 'ArrowLeft' });
    answer([DOWNLOADS]);
    await vi.advanceTimersByTimeAsync(0);

    expect(options()).toHaveLength(1);
  });

  it('drops the search that still waits for its pause when the caret goes to another word', async () => {
    const { findEntries } = setup(vi.fn(), statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS]));

    input().value = 'size of ~/Pob big?';
    input().setSelectionRange(13, 13);
    await fireEvent.input(input());
    input().setSelectionRange(2, 2);
    await fireEvent.keyUp(input(), { key: 'ArrowLeft' });
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);

    expect(findEntries).not.toHaveBeenCalled();
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('completes a folder with white space inside its quotes, then a file in it, and asks it as one path', async () => {
    const SPACED: EntryMatchDto = { node: 7, path: '~/My Files', allocated: 5_000_000, kind: 'dir' };
    const INNER: EntryMatchDto = { node: 8, path: '~/My Files/a b.txt', allocated: 3_000_000, kind: 'file' };
    const askScan = vi.fn().mockResolvedValue(scanAnswer());
    const findEntries = vi.fn().mockResolvedValueOnce([SPACED]).mockResolvedValue([INNER]);

    setup(askScan, statusOf('loaded'), findEntries);

    await typeIn('how big is ~/My');
    await fireEvent.keyDown(input(), { key: 'Tab' });

    expect(input().value).toBe('how big is "~/My Files/"');
    expect(input().selectionStart).toBe(23);

    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);

    expect(findEntries).toHaveBeenLastCalledWith(1, '~/My Files/', FIND_LIMIT);

    await fireEvent.keyDown(input(), { key: 'Tab' });

    expect(input().value).toBe('how big is "~/My Files/a b.txt" ');
    expect(input().selectionStart).toBe(32);

    await fireEvent.submit(input().form as HTMLFormElement);

    expect(askScan).toHaveBeenCalledWith(1, 'how big is "~/My Files/a b.txt"', expect.any(String));
  });

  it('asks the question with Enter when the list is closed', async () => {
    const askScan = vi.fn().mockResolvedValue(scanAnswer());

    setup(askScan, statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS]));

    await typeIn('size of ~/Pob');
    await fireEvent.keyDown(input(), { key: 'Escape' });

    // Enter is not taken, so the browser submits the form.
    const goes = await fireEvent.keyDown(input(), { key: 'Enter' });

    expect(goes).toBe(true);

    await fireEvent.submit(input().form as HTMLFormElement);

    expect(askScan).toHaveBeenCalledWith(1, 'size of ~/Pob', expect.any(String));
  });

  it('lets Enter go on while the list is that of the word before the last key', async () => {
    setup(vi.fn(), statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS]));

    await typeIn('~/Pob');
    await typeIn('~/Pobr', false);

    expect(await fireEvent.keyDown(input(), { key: 'Enter' })).toBe(true);
    expect(input().value).toBe('~/Pobr');
  });

  it('ignores an answer that comes after a later one', async () => {
    const answers: ((found: EntryMatchDto[]) => void)[] = [];
    const findEntries = vi.fn(
      () =>
        new Promise<EntryMatchDto[]>((resolve) => {
          answers.push(resolve);
        }),
    );

    setup(vi.fn(), statusOf('loaded'), findEntries);

    await typeIn('~/Po');
    await typeIn('~/Pob');

    expect(findEntries).toHaveBeenCalledTimes(2);

    answers[1]?.([DOWNLOADS]);
    await vi.advanceTimersByTimeAsync(0);
    answers[0]?.([DOCUMENTS, OLD_ISO]);
    await vi.advanceTimersByTimeAsync(0);

    expect(options().map((option) => option.querySelector('.p')?.textContent)).toEqual(['~/Pobrane/']);
  });

  it('ignores an answer that comes after the question was sent', async () => {
    let answer: (found: EntryMatchDto[]) => void = () => undefined;
    const findEntries = vi.fn(
      () =>
        new Promise<EntryMatchDto[]>((resolve) => {
          answer = resolve;
        }),
    );

    setup(vi.fn().mockResolvedValue(scanAnswer()), statusOf('loaded'), findEntries);

    await typeIn('size of ~/Pob');
    await fireEvent.click(send());
    answer([DOWNLOADS]);
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('makes no call and shows no list while the field is disabled', async () => {
    const { findEntries } = setup(vi.fn(), statusOf('notDownloaded'), vi.fn().mockResolvedValue([DOWNLOADS]));

    await typeIn('~/Pob');

    expect(input().disabled).toBe(true);
    expect(findEntries).not.toHaveBeenCalled();
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('closes the list when the field becomes disabled', async () => {
    const { store } = setup(vi.fn(), statusOf('loaded'), vi.fn().mockResolvedValue([DOWNLOADS]));

    await typeIn('~/Pob');
    expect(screen.getByRole('listbox')).toBeTruthy();

    store.status = statusOf('loaded', { labelling: { scan: 1, done: 2, total: 9 } });
    await vi.advanceTimersByTimeAsync(0);

    expect(input().disabled).toBe(true);
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('closes the list when the field loses the focus, and when the scan is another one', async () => {
    const { rerender, findEntries } = setup(
      vi.fn(),
      statusOf('loaded'),
      vi.fn().mockResolvedValue([DOWNLOADS]),
    );

    await typeIn('~/Pob');
    expect(screen.getByRole('listbox')).toBeTruthy();

    await fireEvent.blur(input());
    expect(screen.queryByRole('listbox')).toBeNull();

    await typeIn('~/Pobr');
    expect(screen.getByRole('listbox')).toBeTruthy();

    await rerender({ scan: 2 });
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(input().value).toBe('');
    expect(findEntries).toHaveBeenCalledTimes(2);
  });

  it('leaves the field without a list when the search fails', async () => {
    setup(vi.fn(), statusOf('loaded'), vi.fn().mockRejectedValue(new ApiRequestError('io', 'failed')));

    await typeIn('~/Pob');

    expect(screen.queryByRole('listbox')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });
});
