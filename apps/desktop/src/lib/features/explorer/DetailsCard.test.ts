import { render, screen } from '@testing-library/svelte';
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
