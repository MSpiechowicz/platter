import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { settings } from '../../state/settings.svelte';
import { basisOf } from './model';
import ResultPanel from './ResultPanel.svelte';
import { cleanGroup, fixture, finish, GB, type Fixture } from './testFixture';

beforeEach(() => {
  settings.set('units', 'GB');
});

const basis = { used: 1408 * GB, capacity: 1900 * GB };

/** A store whose run has ended with `extra` as its last event. */
async function ended(extra: Parameters<typeof finish>[2] = {}): Promise<Fixture> {
  const context = await fixture();

  await context.store.cleanSelected();
  cleanGroup(context.send, 'target');
  finish(context.send, context.scanId, extra);

  return context;
}

const bars = (): { label: string; value: number }[] =>
  screen.getAllByRole('progressbar').map((bar) => ({
    label: bar.getAttribute('aria-label') ?? '',
    value: Number(bar.getAttribute('aria-valuenow')),
  }));

describe('ResultPanel', () => {
  it('shows what was freed, and the used space before and after as real numbers', async () => {
    const context = await ended({ freed: 252 * GB });

    render(ResultPanel, { props: { store: context.store, basis } });

    expect(screen.getByRole('heading', { name: 'Result' })).toBeTruthy();
    expect(screen.getByText('252 GB')).toBeTruthy();
    expect(screen.getByText('freed')).toBeTruthy();
    expect(screen.getByText('1.41 TB used · 74%')).toBeTruthy();
    expect(screen.getByText('1.16 TB used · 61%')).toBeTruthy();
    expect(bars()).toEqual([
      { label: 'Before the cleanup', value: 74 },
      { label: 'After the cleanup', value: 61 },
    ]);
  });

  it('says so when the freed space could not be measured, and draws no after bar', async () => {
    const context = await ended({ freed: undefined });

    render(ResultPanel, { props: { store: context.store, basis } });

    expect(screen.getByText('Freed: not measured')).toBeTruthy();
    expect(screen.getByText('Not measured')).toBeTruthy();
    expect(bars().map((bar) => bar.value)).toEqual([74, 0]);
    expect(screen.getByRole('progressbar', { name: 'After the cleanup: not measured' })).toBeTruthy();
  });

  it('measures a folder by what the scan counted', async () => {
    const context = await ended({ freed: 25 * GB });

    render(ResultPanel, { props: { store: context.store, basis: basisOf(undefined, 100 * GB) } });

    expect(screen.getByText('100 GB scanned · 100%')).toBeTruthy();
    expect(screen.getByText('75 GB scanned · 75%')).toBeTruthy();
  });

  it('says that cleaning failed, and offers no number', async () => {
    const context = await fixture();

    await context.store.cleanSelected();
    context.send({
      type: 'failed',
      message: 'the disk is full',
      outdatedScan: context.scanId,
      canUndo: false,
      cleanupId: 77,
    });
    render(ResultPanel, { props: { store: context.store, basis } });

    expect(screen.getByText('Cleaning failed')).toBeTruthy();
    expect(screen.getByRole('status').textContent).toContain('the disk is full');
    expect(screen.getByText('Not measured')).toBeTruthy();
  });

  it('offers Undo only when something can be put back, and "Scan again" always', async () => {
    const without = await ended({ canUndo: false });

    render(ResultPanel, { props: { store: without.store, basis } });

    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Scan again' })).toBeTruthy();
  });

  it('asks the store to undo when Undo is clicked', async () => {
    const context = await ended({ canUndo: true });
    const undo = vi.spyOn(context.store, 'undo');

    render(ResultPanel, { props: { store: context.store, basis } });
    await fireEvent.click(screen.getByRole('button', { name: 'Undo' }));

    expect(undo).toHaveBeenCalledOnce();
  });

  it('scans again through the store', async () => {
    const context = await ended();
    const rescan = vi.spyOn(context.store, 'rescan').mockResolvedValue();

    render(ResultPanel, { props: { store: context.store, basis } });
    await fireEvent.click(screen.getByRole('button', { name: 'Scan again' }));

    expect(rescan).toHaveBeenCalledOnce();
  });

  it('shows problems as plain text with control characters made visible', async () => {
    const context = await fixture();

    await context.store.cleanSelected();
    context.send({
      type: 'targetDone',
      ruleId: 'target',
      path: '/home/demo/evil‮file',
      allocated: 1,
      outcome: { kind: 'failed', detail: 'busy', problems: 0 },
    });
    finish(context.send, context.scanId, { withProblems: 1 });
    render(ResultPanel, { props: { store: context.store, basis } });

    expect(screen.getByText(/evil\\u\{202E\}file: busy/)).toBeTruthy();
  });

  it('shows nothing before a cleanup has ended', async () => {
    const context = await fixture();

    render(ResultPanel, { props: { store: context.store, basis } });

    expect(screen.queryByRole('button', { name: 'Scan again' })).toBeNull();
  });
});
