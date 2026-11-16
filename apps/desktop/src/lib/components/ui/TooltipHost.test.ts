import { flushSync } from 'svelte';
import { render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { tip } from './tooltip.svelte';
import TooltipHost from './TooltipHost.svelte';

describe('TooltipHost', () => {
  afterEach(() => {
    tip.target = null;
  });

  it('shows a list that repeats a line, such as release notes, without throwing on the duplicate', () => {
    render(TooltipHost);

    tip.show(document.body, { title: 'Update', items: ['Fixes', 'Fixes', 'Speed'] });
    flushSync();

    expect(screen.getAllByText('Fixes')).toHaveLength(2);
    expect(screen.getByText('Speed')).toBeTruthy();
  });
});
