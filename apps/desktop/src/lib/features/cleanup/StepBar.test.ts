import { render, screen, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import StepBar from './StepBar.svelte';

const steps = (): HTMLElement[] =>
  within(screen.getByRole('list', { name: 'Steps' })).getAllByRole('listitem');

describe('StepBar', () => {
  it('lists Scan, Review and Clean, and marks the current one', () => {
    render(StepBar, { props: { step: 'review' } });

    expect(steps().map((item) => item.querySelector(':scope > span')?.textContent)).toEqual([
      'Scan',
      'Review',
      'Clean',
    ]);
    expect(steps().map((item) => item.getAttribute('aria-current'))).toEqual([null, 'step', null]);
  });

  it('numbers the steps that are not done, and ticks the ones that are', () => {
    render(StepBar, { props: { step: 'clean' } });

    const [scan, review, clean] = steps();

    expect(scan?.querySelector('.dot')?.textContent?.trim()).toBe('');
    expect(scan?.querySelector('.dot svg')).toBeTruthy();
    expect(review?.querySelector('.dot svg')).toBeTruthy();
    expect(clean?.querySelector('.dot')?.textContent?.trim()).toBe('3');
    expect(clean?.getAttribute('aria-current')).toBe('step');
  });

  it('ticks every step, with no current one, once the cleanup is over', () => {
    render(StepBar, { props: { step: 'clean', complete: true } });

    expect(steps().map((item) => item.querySelector('.dot svg') !== null)).toEqual([true, true, true]);
    expect(steps().map((item) => item.getAttribute('aria-current'))).toEqual([null, null, null]);
  });

  it('is on the first step with its number while a scan runs', () => {
    render(StepBar, { props: { step: 'scan' } });

    expect(steps()[0]?.querySelector('.dot')?.textContent?.trim()).toBe('1');
    expect(steps()[0]?.getAttribute('aria-current')).toBe('step');
  });
});
