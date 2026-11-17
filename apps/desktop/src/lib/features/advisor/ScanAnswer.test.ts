import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { AnswerSegmentDto, ScanAnswerDto } from '../../api/types';
import { scanAnswer } from './fixtures';
import ScanAnswer from './ScanAnswer.svelte';

const RLO = String.fromCharCode(0x202e);

const text = (value: string): AnswerSegmentDto => ({ kind: 'text', text: value });
const path = (
  node: number,
  name: string,
  verdict?: 'safe' | 'review' | 'keep',
  source?: 'guard' | 'rule' | 'user' | 'model',
): AnswerSegmentDto => ({
  kind: 'path',
  node,
  path: name,
  ...(verdict ? { verdict } : {}),
  ...(source ? { source } : {}),
});

function show(extra: Partial<ScanAnswerDto> = {}, onopen = vi.fn()) {
  const view = render(ScanAnswer, { props: { answer: scanAnswer(extra), onopen, onask: vi.fn() } });
  const paragraph = view.container.querySelector('p.text') as HTMLElement;

  return { ...view, onopen, paragraph };
}

/** The paragraph as the reader meets it: text pieces and chips (by their accessible name), in order. */
function pieces(paragraph: HTMLElement): string[] {
  return Array.from(paragraph.childNodes)
    .map((node) =>
      node instanceof HTMLElement ? `[${node.getAttribute('aria-label')}]` : (node.textContent ?? '').trim(),
    )
    .filter((piece) => piece !== '');
}

describe('ScanAnswer: the answer as text and chips', () => {
  it('writes the segments in order, a path as a chip between its texts', () => {
    const { paragraph } = show({
      answer: 'ignored',
      segments: [
        text('A '),
        path(1, '/var/cache/pacman/pkg', 'review', 'rule'),
        text(' needs your review. '),
        path(2, '/var/cache/apt/archives', 'safe', 'rule'),
        text(' (210 MB) is safe to clean.'),
      ],
    });

    expect(pieces(paragraph)).toEqual([
      'A',
      '[/var/cache/pacman/pkg, needs review]',
      'needs your review.',
      '[/var/cache/apt/archives, safe to clean]',
      '(210 MB) is safe to clean.',
    ]);
    expect(screen.queryByText('ignored')).toBeNull();
  });

  it('keeps the spaces of the texts around a chip, so the sentence reads as written', () => {
    const { paragraph } = show({
      segments: [text('See '), path(1, '/a', 'keep', 'guard'), text(' now.')],
    });

    expect(paragraph.textContent?.replace(/\s+/g, ' ')).toMatch(/^\s*See \/a keep now\.\s*$/);
  });

  it.each([
    ['safe', 'safe to clean'],
    ['review', 'needs review'],
    ['keep', 'kept'],
    [undefined, 'no verdict'],
  ] as const)(
    'names a chip with the verdict %s "%s" and marks it in the colour of that verdict',
    (verdict, words) => {
      const { paragraph } = show({
        segments: [path(7, '/home/demo/x', verdict, verdict ? 'rule' : undefined)],
      });
      const chip = screen.getByRole('button', { name: `/home/demo/x, ${words}` });
      const mark = chip.querySelector('.mark') as HTMLElement;

      expect(paragraph.contains(chip)).toBe(true);
      expect(mark.classList.contains(verdict ?? 'none')).toBe(true);
      expect(mark.textContent?.trim()).toBe(verdict ?? '');
      expect(mark.getAttribute('aria-hidden')).toBe('true');
    },
  );

  it('opens the folder of a chip in the Explorer when it is clicked', async () => {
    const { onopen } = show({ segments: [text('In '), path(42, '/home/demo/.cache', 'safe', 'rule')] });

    await fireEvent.click(screen.getByRole('button', { name: '/home/demo/.cache, safe to clean' }));

    expect(onopen).toHaveBeenCalledTimes(1);
    expect(onopen).toHaveBeenCalledWith(42);
  });

  it('keeps the whole path in the tooltip and in the cut text, and makes hidden characters of a name visible', () => {
    const hostile = `/home/demo/a${RLO}gnp.exe`;
    const long = `/srv/${'deeply-nested-folder/'.repeat(8)}end`;

    show({ segments: [path(1, hostile, 'keep', 'guard'), text(' and '), path(2, long)] });

    const hostileChip = screen.getByRole('button', { name: '/home/demo/a\\u{202E}gnp.exe, kept' });
    const longChip = screen.getByRole('button', { name: `${long}, no verdict` });

    expect(hostileChip.querySelector('.cp')?.textContent).toBe('/home/demo/a\\u{202E}gnp.exe');
    expect(hostileChip.title).toContain('/home/demo/a\\u{202E}gnp.exe');
    expect(hostileChip.textContent).not.toContain(RLO);
    expect(longChip.querySelector('.cp')?.textContent).toBe(long);
    expect(longChip.title).toContain(long);
  });

  it('shows hidden characters of a text segment too', () => {
    const { paragraph } = show({ segments: [text(`Bidi ${RLO}gnp.exe`)] });

    expect(paragraph.textContent).toContain('Bidi \\u{202E}gnp.exe');
    expect(paragraph.textContent).not.toContain(RLO);
  });

  it('writes the plain answer when there are no segments', () => {
    const { paragraph } = show({ answer: 'Facts only: “/a” (1 GB).', segments: [] });

    expect(paragraph.textContent).toBe('Facts only: “/a” (1 GB).');
    expect(paragraph.querySelector('button')).toBeNull();
  });
});

describe('ScanAnswer: what is under the text', () => {
  it('shows no "Understood as" line and no note on who wrote the answer', () => {
    show({ understood: { intent: 'cleanable', kind: 'any', limit: 5, scopeText: 'docker' } });

    expect(
      screen.queryByText(/Understood as|Answer written by Platter|Written by the local model/),
    ).toBeNull();
  });
});

describe('ScanAnswer: notes', () => {
  it('lists a note once: not again under a text that already says it', () => {
    const { container } = show({
      answer: 'Safe to clean: 1 GB. 2 matching files are counted in the totals but not listed.',
      segments: [text('Safe to clean: 1 GB. 2 matching files are counted in the totals but not listed.')],
      notes: [
        '2 matching files are counted in the totals but not listed',
        'This scan is from before a cleanup.',
      ],
    });

    const listed = Array.from(container.querySelectorAll('ul.notes li')).map((item) => item.textContent);

    expect(listed).toEqual(['This scan is from before a cleanup.']);
  });
});

describe('ScanAnswer: notes with a path', () => {
  it('lists a note that names a path once, though the text sets the path in quotes', () => {
    const { container } = show({
      answer: 'Nothing to list for this scan. No folder called “Steam” is in this scan.',
      segments: [text('Nothing to list for this scan. No folder called “Steam” is in this scan.')],
      notes: ["No folder called 'Steam' is in this scan", 'This scan is from before a cleanup.'],
    });

    const listed = Array.from(container.querySelectorAll('ul.notes li')).map((item) => item.textContent);

    expect(listed).toEqual(['This scan is from before a cleanup.']);
  });
});

describe('ScanAnswer: a cleanable answer about a kept folder', () => {
  it('shows the kept folder as a chip and as a row with the keep tag, and lists its note once', () => {
    const answer =
      'No: \u201C/home/demo/Documents\u201D (12 GB) is left alone by Platter. This is a protected place, or inside one. Platter does not clean it.';
    const { container, paragraph } = show({
      answer,
      understood: {
        intent: 'cleanable',
        kind: 'any',
        limit: 5,
        scope: { node: 21, path: '/home/demo/Documents' },
      },
      segments: [
        text('No: '),
        path(21, '/home/demo/Documents', 'keep', 'guard'),
        text(
          ' (12 GB) is left alone by Platter. This is a protected place, or inside one. Platter does not clean it.',
        ),
      ],
      rows: [
        {
          node: 21,
          path: '/home/demo/Documents',
          allocated: 12_000_000_000,
          verdict: 'keep',
          source: 'guard',
          note: 'Left alone by the safety guard, not decided by the model.',
        },
      ],
      notes: ['This is a protected place, or inside one. Platter does not clean it.'],
    });
    const row = container.querySelector('li.row') as HTMLElement;

    expect(pieces(paragraph)[0]).toBe('No:');
    expect(screen.getByRole('button', { name: '/home/demo/Documents, kept' })).toBeTruthy();
    expect(row.querySelector('.meta')?.textContent).toMatch(
      /keep\s*Safety guard\s*Left alone by the safety guard/,
    );
    expect(container.querySelector('ul.notes')).toBeNull();
  });
});

describe('ScanAnswer: a question asked back', () => {
  const prompt = 'I found more than one folder called \u201Cbackup\u201D. Which one do you mean?';
  const clarify = (extra: Partial<ScanAnswerDto> = {}): ScanAnswerDto =>
    scanAnswer({
      answer: prompt,
      segments: [text(prompt)],
      rows: [],
      notes: ['Never shown next to a question asked back'],
      clarify: {
        prompt,
        choices: [
          { id: 0, label: '~/work/backup', path: '~/work/backup', node: 21 },
          { id: 1, label: 'Go module cache' },
        ],
      },
      ...extra,
    });

  function showClarify(extra: Partial<ScanAnswerDto> = {}, props: { pickDisabled?: boolean } = {}) {
    const onpick = vi.fn();
    const view = render(ScanAnswer, {
      props: { answer: clarify(extra), onopen: vi.fn(), onask: vi.fn(), onpick, ...props },
    });

    return { ...view, onpick };
  }

  it('is the prompt and one button per choice, with no figures, notes or rows', () => {
    showClarify();

    expect(screen.getByText(prompt)).toBeTruthy();
    expect(screen.getAllByRole('button').map((button) => button.textContent?.trim())).toEqual([
      '~/work/backup',
      'Go module cache',
    ]);
    expect(document.querySelector('.notes, .rows, .more')).toBeNull();
    expect(screen.queryByText(/Understood as|Answer written by Platter/)).toBeNull();
  });

  it('gives the choice to the one who asked when a button is pressed', async () => {
    const { onpick } = showClarify();

    await fireEvent.click(screen.getByRole('button', { name: 'Go module cache' }));
    await fireEvent.click(screen.getByRole('button', { name: '~/work/backup' }));

    expect(onpick.mock.calls).toEqual([
      [{ id: 1, label: 'Go module cache' }],
      [{ id: 0, label: '~/work/backup', path: '~/work/backup', node: 21 }],
    ]);
  });

  it('makes every button native and keyboard focusable, and shuts them all while a question is on its way', () => {
    showClarify({}, { pickDisabled: true });

    for (const button of screen.getAllByRole('button') as HTMLButtonElement[]) {
      expect(button.tagName).toBe('BUTTON');
      expect(button.type).toBe('button');
      expect(button.disabled).toBe(true);
    }
  });

  it('shows hidden characters of a choice, in its label and in its tooltip', () => {
    showClarify({
      clarify: {
        prompt,
        choices: [{ id: 0, label: `~/a${RLO}b`, path: `~/a${RLO}b`, node: 3 }],
      },
    });

    const button = screen.getByRole('button') as HTMLButtonElement;

    expect(button.textContent).toContain('~/a\\u{202E}b');
    expect(button.title).toBe('~/a\\u{202E}b: ask about this one');
    expect(document.body.innerHTML).not.toContain(RLO);
  });

  it('writes an answer that is not asked back as it did, with its figures and rows', () => {
    render(ScanAnswer, { props: { answer: scanAnswer(), onopen: vi.fn(), onask: vi.fn() } });

    expect(screen.queryByText(/Understood as/)).toBeNull();
    expect(document.querySelector('.rows')).toBeTruthy();
    expect(screen.queryByRole('list', { name: 'Which one do you mean?' })).toBeNull();
  });
});

describe('ScanAnswer: rows of files', () => {
  const rows = [
    { node: 1, path: '~/Downloads', allocated: 96_000_000_000 },
    { node: 2, path: '~/Downloads/old.iso', allocated: 4_200_000_000, file: true as const },
  ];

  it('shows no Ask on a file row, and keeps its Open and the Ask of a folder row', async () => {
    const onopen = vi.fn();
    const onask = vi.fn();

    render(ScanAnswer, { props: { answer: scanAnswer({ rows, more: 0 }), onopen, onask } });

    expect(screen.queryByRole('button', { name: 'Ask about ~/Downloads/old.iso' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Ask about ~/Downloads' })).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Open ~/Downloads/old.iso in the Explorer' }));

    expect(onopen).toHaveBeenCalledWith(2);
    expect(screen.getByText('4.2 GB')).toBeTruthy();
  });
});
