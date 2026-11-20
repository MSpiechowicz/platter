import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { settings } from '../../state/settings.svelte';
import OutdatedHero from './OutdatedHero.svelte';

const NOW = new Date(2026, 10, 17, 12).getTime();

describe('OutdatedHero', () => {
  beforeEach(() => {
    settings.reduceMotion = true;
    settings.set('units', 'GB');
    vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
    vi.setSystemTime(NOW);
  });

  afterEach(() => {
    cleanup();
    settings.reduceMotion = false;
    vi.useRealTimers();
  });

  it('names the heading, the picture and the way on, and has no button', () => {
    render(OutdatedHero, { props: {} });

    expect(screen.getByRole('img', { name: /old scan/ })).toBeTruthy();
    expect(screen.getByText('The scan is out of date')).toBeTruthy();
    expect(screen.getByText('Rescan from the banner above to see what is left.')).toBeTruthy();
    expect(screen.queryAllByRole('button')).toEqual([]);
  });

  it('shows an Undo button under the note only when it is given something to call, and calls it', async () => {
    const onundo = vi.fn();

    render(OutdatedHero, { props: { onundo } });

    const undo = screen.getByRole('button', { name: 'Undo' });

    expect(screen.queryAllByRole('button')).toEqual([undo]);
    expect(screen.queryByRole('button', { name: 'Rescan' })).toBeNull();

    await fireEvent.click(undo);

    expect(onundo).toHaveBeenCalledOnce();
  });

  it('keeps the keyboard on the card when Undo goes away after the click', async () => {
    const { rerender } = render(OutdatedHero, { props: { onundo: () => {} } });

    const undo = screen.getByRole('button', { name: 'Undo' });

    undo.focus();
    await fireEvent.click(undo);
    // The store takes Undo off once it is clicked.
    await rerender({ onundo: undefined, undone: ['Put back 1 item.'] });

    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole('status'));
    expect(document.activeElement?.textContent).toContain('Put back 1 item.');
  });

  it('lists how an undo went, as plain text', () => {
    render(OutdatedHero, { props: { undone: ['Put back 1 item.', 'Not put back: ~/b (already exists)'] } });

    expect(screen.getByRole('status').textContent).toContain('Put back 1 item.');
    expect(screen.getByRole('status').textContent).toContain('Not put back: ~/b (already exists)');
  });

  it('says what was freed and how long ago, and keeps the time current by the minute', async () => {
    render(OutdatedHero, { props: { freed: 27_000_000_000, endedAt: NOW - 2 * 60_000 } });

    expect(screen.getByText(/^You cleaned 27 GB 2 minutes ago\./)).toBeTruthy();

    vi.advanceTimersByTime(60_000 * 3);
    await tick();

    expect(screen.getByText(/^You cleaned 27 GB 5 minutes ago\./)).toBeTruthy();
  });

  it('uses no semicolon in its words', () => {
    render(OutdatedHero, {
      props: { freed: 27_000_000_000, endedAt: NOW, onundo: () => {}, undone: ['Put back 1 item.'] },
    });

    expect(document.body.textContent).not.toContain(';');
  });

  it('falls back to general words without the freed size or the time, or when nothing was freed', () => {
    for (const props of [{}, { freed: 27_000_000_000 }, { endedAt: NOW }, { freed: 0, endedAt: NOW }]) {
      render(OutdatedHero, { props });

      expect(screen.getByText(/^A cleanup changed the disk after this scan\./)).toBeTruthy();
      expect(screen.queryByText(/You cleaned/)).toBeNull();
      cleanup();
    }
  });

  it('adds the animation class only while motion is allowed', () => {
    render(OutdatedHero, { props: {} });
    expect(screen.getByRole('img').classList.contains('enter')).toBe(false);
    cleanup();

    settings.reduceMotion = false;
    render(OutdatedHero, { props: {} });
    expect(screen.getByRole('img').classList.contains('enter')).toBe(true);
  });
});
