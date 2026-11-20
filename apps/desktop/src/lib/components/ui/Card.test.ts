import { render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Card from './Card.svelte';

const body = createRawSnippet(() => ({ render: () => '<p>Body</p>' }));

describe('Card', () => {
  it('shows its hint in the header', () => {
    render(Card, { props: { title: 'Sizes', hint: 'by space used', children: body } });

    expect(screen.getByRole('heading', { name: 'Sizes by space used' })).toBeTruthy();
  });

  it('shows a tag in the place of the hint, not beside it', () => {
    render(Card, { props: { title: 'Sizes', hint: 'by space used', tag: 'OLD SCAN', children: body } });

    expect(screen.getByRole('heading', { name: 'Sizes OLD SCAN' })).toBeTruthy();
    expect(screen.queryByText('by space used')).toBeNull();
  });
});
