import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { NodeDto } from '../../api/types';
import DetailsCard from './DetailsCard.svelte';

const node: NodeDto = {
  id: 4,
  name: 'opt',
  kind: 'dir',
  allocated: 3_600_000_000,
  apparent: 3_600_000_000,
  files: 120,
  hasChildren: true,
  category: 'app',
  mtime: 1_700_000_000,
};

const props = (extra: object) => ({
  node,
  selected: true,
  path: '/opt',
  canOpen: true,
  onopen: vi.fn(),
  onreveal: vi.fn(),
  ...extra,
});

describe('DetailsCard share', () => {
  it('says how much of the open folder the selected item holds', () => {
    render(DetailsCard, { props: props({ share: 0.0023 }) });

    expect(screen.getByText('0.2% of this folder')).toBeTruthy();
  });

  it('says nothing when the card is about the open folder itself', () => {
    render(DetailsCard, { props: props({ selected: false }) });

    expect(screen.queryByText(/of this folder/)).toBeNull();
  });

  it('shows the colour of the item as a dot', () => {
    render(DetailsCard, { props: props({ color: '#3ec5d8' }) });

    expect(document.querySelector('.dot')?.getAttribute('style')).toContain('#3ec5d8');
  });
});

describe('DetailsCard buttons', () => {
  const names = () =>
    [...document.querySelectorAll('.actions button')].map(
      (button) => button.textContent?.trim() || button.getAttribute('aria-label'),
    );

  it('always has "Go into folder", "Mark for cleanup" and the file manager, whether or not they can be used', () => {
    render(DetailsCard, { props: props({ node: undefined, selected: false, canOpen: false }) });

    expect(names()).toEqual(['Go into folder', 'Mark for cleanup', 'Show in file manager']);
    expect((screen.getByRole('button', { name: 'Go into folder' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'Mark for cleanup' }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it('marks on a click, when there is a way to', async () => {
    const onmark = vi.fn();

    render(DetailsCard, { props: props({ onmark, markOff: undefined }) });
    await fireEvent.click(screen.getByRole('button', { name: 'Mark for cleanup' }));

    expect(onmark).toHaveBeenCalledOnce();
  });

  it('says "Unmark" for what is marked, in the plain button', () => {
    render(DetailsCard, { props: props({ onmark: vi.fn(), markOff: undefined, marked: true }) });

    const button = screen.getByRole('button', { name: 'Unmark' });

    expect(button.classList.contains('primary')).toBe(false);
    expect(screen.queryByRole('button', { name: 'Mark for cleanup' })).toBeNull();
  });

  it('is off with the reason as its tooltip', () => {
    render(DetailsCard, { props: props({ onmark: vi.fn(), markOff: 'Kept on purpose.' }) });

    const button = screen.getByRole('button', { name: 'Mark for cleanup' }) as HTMLButtonElement;

    expect(button.disabled).toBe(true);
    expect(button.title).toBe('Kept on purpose.');
  });

  it('shows a path with its hidden characters made visible', () => {
    render(DetailsCard, { props: props({ path: '/tmp/a‮b' }) });

    expect(screen.getByRole('heading', { name: '/tmp/a\\u{202E}b' })).toBeTruthy();
  });
});
