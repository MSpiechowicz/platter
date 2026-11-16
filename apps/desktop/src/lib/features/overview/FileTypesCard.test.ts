import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { Category, SummaryDto } from '../../api/types';
import FileTypesCard from './FileTypesCard.svelte';

const GB = 1_000_000_000;

const CATEGORIES: Category[] = [
  'video',
  'app',
  'archive',
  'image',
  'code',
  'package',
  'document',
  'audio',
  'diskImage',
  'build',
  'other',
];

/** `count` categories, the first one the biggest. */
function categories(count: number): SummaryDto['byCategory'] {
  return CATEGORIES.slice(0, count).map((category, index) => ({
    category,
    allocated: (100 - index * 8) * GB,
    files: 10,
  }));
}

describe('FileTypesCard', () => {
  it('lists a category that holds only directory blocks, with no files, next to the big ones', () => {
    const byCategory: SummaryDto['byCategory'] = [
      { category: 'video', allocated: 8_000_000_000, files: 12 },
      { category: 'build', allocated: 4096, files: 0 },
    ];

    render(FileTypesCard, { byCategory });

    expect(screen.getByText('Video')).toBeTruthy();
    expect(screen.getByText('8.0 GB')).toBeTruthy();
    expect(screen.getByText('Build artifacts')).toBeTruthy();
    expect(screen.getByText('4.1 KB')).toBeTruthy();
    expect(
      screen.getByRole('progressbar', { name: 'Build artifacts share' }).getAttribute('aria-valuenow'),
    ).toBe('0');
  });

  it('shows the size of every row, and its share as a hint on the row', () => {
    render(FileTypesCard, {
      byCategory: [
        { category: 'video', allocated: 909 * GB, files: 1 },
        { category: 'image', allocated: 1091 * GB, files: 1 },
      ],
    });

    expect(screen.getByText('1.09 TB').closest('.type')?.getAttribute('title')).toBe(
      '55% of the scanned space',
    );
    expect(screen.getByText('909 GB').closest('.type')?.getAttribute('title')).toBe(
      '45% of the scanned space',
    );
  });

  it('keeps the eight biggest categories and folds the rest into "Smaller types"', () => {
    render(FileTypesCard, { byCategory: categories(11) });

    expect(screen.getAllByRole('progressbar')).toHaveLength(9);
    expect(screen.queryByText('Other')).toBeNull();
    expect(screen.getByRole('group', { name: /^Smaller types: .*Other/ })).toBeTruthy();
  });

  it('lists every category when there are no more than eight, and when folding would save no row', () => {
    const { unmount } = render(FileTypesCard, { byCategory: categories(8) });

    expect(screen.queryByText('Smaller types')).toBeNull();
    unmount();

    render(FileTypesCard, { byCategory: categories(9) });

    expect(screen.getAllByRole('progressbar')).toHaveLength(9);
    expect(screen.queryByText('Smaller types')).toBeNull();
  });

  it('does not stretch its rows to fill the card', () => {
    const { container } = render(FileTypesCard, { byCategory: categories(11) });
    const list = container.querySelector('.types') as HTMLElement;
    const rows = [...list.querySelectorAll<HTMLElement>('.type')];

    expect(rows).toHaveLength(9);

    // Neither the list nor a row may carry inline sizing, and the card is not the one that takes the spare height.
    expect(list.getAttribute('style')).toBeNull();
    for (const row of rows) {
      expect(row.getAttribute('style')).toBeNull();
    }
    expect(container.querySelector('section')?.classList.contains('grow')).toBe(false);
  });
});
