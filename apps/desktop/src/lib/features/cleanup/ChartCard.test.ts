import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { settings } from '../../state/settings.svelte';
import ChartCard from './ChartCard.svelte';
import { basisOf, type Basis } from './model';
import { cleanGroup, DATA, finish, fixture, GB, stubChartBox, type Fixture } from './testFixture';

let restore = (): void => {};

beforeEach(() => {
  settings.set('units', 'GB');
  settings.reduceMotion = true;
  restore = stubChartBox();
});

afterEach(() => {
  settings.reduceMotion = false;
  restore();
});

const USED = 1408 * GB;
const CAPACITY = 1900 * GB;

function show(context: Fixture, basis: Basis = { used: USED, capacity: CAPACITY }): void {
  render(ChartCard, { props: { store: context.store, basis } });
}

const row = (name: RegExp | string): HTMLElement => screen.getByRole('checkbox', { name });
const hero = (): HTMLElement => document.querySelector('.hero') as HTMLElement;
const ribbons = (): SVGPathElement[] => [...document.querySelectorAll<SVGPathElement>('path.ribbon')];

describe('ChartCard in the Review step', () => {
  it('shows what the selection frees and what the disk looks like after it', async () => {
    const context = await fixture();

    show(context);

    // The two safe rows that are bigger than nothing: 88 + 17 + 0.4 GB.
    expect(within(hero()).getByText('Selected to free')).toBeTruthy();
    expect(within(hero()).getByText('105 GB')).toBeTruthy();
    expect(within(hero()).getByText('Disk after cleanup')).toBeTruthy();
    expect(within(hero()).getByText('1.30 TB used · 69%')).toBeTruthy();
  });

  it('says only what is known for a folder: no disk, no share of one', async () => {
    const context = await fixture();

    show(context, { used: USED, capacity: undefined });

    expect(within(hero()).getByText('Folder after cleanup')).toBeTruthy();
    expect(within(hero()).getByText('1.30 TB scanned')).toBeTruthy();
    expect(hero().textContent).not.toContain('%');
  });

  it('draws one row per group with its size, largest first, and "Everything else" last', async () => {
    const context = await fixture();

    show(context);

    const names = [...document.querySelectorAll('.rows .row .name')].map((name) => name.textContent);

    expect(names).toEqual([
      'Title of snapshots',
      'Title of steam (kept)',
      'Title of target',
      'Title of trash',
      'Title of thin',
      'Everything else',
    ]);
    expect(within(row('Title of snapshots, 268 GB')).getByText('268 GB')).toBeTruthy();
  });

  it('ticks the selected groups, offers only the groups that can be cleaned, and never a kept one', async () => {
    const context = await fixture();

    show(context);

    expect(row(/^Title of target/).getAttribute('aria-checked')).toBe('true');
    expect(row(/^Title of snapshots/).getAttribute('aria-checked')).toBe('false');
    expect(screen.queryByRole('checkbox', { name: /steam/ })).toBeNull();
  });

  it('moves a group between the sides with a click on its row, exactly as its checkbox in the list does', async () => {
    const context = await fixture();

    show(context);
    await fireEvent.click(row(/^Title of snapshots/));

    expect(context.store.isSelected('snapshots')).toBe(true);
    expect(row(/^Title of snapshots/).getAttribute('aria-checked')).toBe('true');
    expect(within(hero()).getByText('373 GB')).toBeTruthy();

    await fireEvent.click(row(/^Title of snapshots/));

    expect(context.store.isSelected('snapshots')).toBe(false);
  });

  it('moves a group with a click on its ribbon too', async () => {
    const context = await fixture();

    show(context);

    // The first ribbon belongs to the biggest group, the snapshots, which is not selected.
    await fireEvent.click(ribbons()[0] as SVGPathElement);

    expect(context.store.isSelected('snapshots')).toBe(true);
  });

  it('keeps a thin group readable: it has a row of its own that can be clicked', async () => {
    const context = await fixture();

    show(context);

    const thin = row(/^Title of thin/);

    expect(thin.querySelector('.name')?.textContent).toBe('Title of thin');
    expect(Number.parseFloat((thin as HTMLElement).style.height)).toBeGreaterThanOrEqual(16);
  });

  it('draws the rest of the disk as one more group, not selectable', async () => {
    const context = await fixture();

    show(context);

    expect(screen.queryByRole('checkbox', { name: /Everything else/ })).toBeNull();
    expect(screen.getByText('Everything else')).toBeTruthy();
  });

  it('names the two bars on the right with what is freed and what stays', async () => {
    const context = await fixture();

    show(context);

    const ends = [...document.querySelectorAll('.endtext')].map((end) =>
      end.textContent?.replace(/\s+/g, ' ').trim(),
    );

    expect(ends).toEqual(['WILL BE FREED 105 GB', 'STAYS ON DISK 1.30 TB']);
  });

  it('takes the clicks away while a cleanup is being planned or reviewed', async () => {
    const context = await fixture();

    show(context);
    await context.store.cleanSelected();

    expect(screen.queryAllByRole('checkbox')).toHaveLength(0);
    // The selection is still drawn.
    expect(document.querySelector('.endtext')?.textContent).toContain('105 GB');
  });

  it('shows the legend and the hint for the clicks', async () => {
    const context = await fixture();

    show(context);

    expect(screen.getByText('Click a ribbon to move it between the two sides')).toBeTruthy();
    expect(screen.getByText('kept')).toBeTruthy();
  });

  it('shows control and bidi characters of a title as text', async () => {
    const context = await fixture();

    context.store.load(context.scanId, {
      ...DATA,
      suggestions: DATA.suggestions.map((item) =>
        item.id === 'target' ? { ...item, title: 'Evil‮doc' } : item,
      ),
    });
    show(context);

    expect(screen.getByRole('checkbox', { name: /^Evil\\u\{202E\}doc/ })).toBeTruthy();
  });

  it('draws nothing but a note when the scan found no bytes', async () => {
    const context = await fixture();

    context.store.load(context.scanId, { suggestions: [], totals: { safe: 0, review: 0, keep: 0 } });
    render(ChartCard, { props: { store: context.store, basis: basisOf(undefined, 0) } });

    expect(screen.getByText(/Nothing to show/)).toBeTruthy();
  });
});

describe('ChartCard in the Clean step', () => {
  async function running(): Promise<Fixture> {
    const context = await fixture();

    show(context);
    await context.store.cleanSelected();

    return context;
  }

  const nameOf = (id: string): HTMLElement | null =>
    [...document.querySelectorAll<HTMLElement>('.rows .row')]
      .find((item) => item.textContent?.includes(`Title of ${id}`))
      ?.querySelector('.name') ?? null;

  it('turns the ribbons of the finished groups solid one by one', async () => {
    const context = await running();
    const opacity = (index: number): string | null => ribbons()[index]?.getAttribute('fill-opacity') ?? null;

    // Before any group is done: the selected ones are drawn at the usual strength.
    expect(context.store.phase.kind).toBe('running');
    expect(opacity(2)).toBe('0.5');

    cleanGroup(context.send, 'target');

    // `target` is the third bar from the top.
    await Promise.resolve();
    expect(opacity(2)).toBe('0.8');
    expect(opacity(3)).toBe('0.5');
    expect(nameOf('target')?.classList.contains('gone')).toBe(true);
    expect(nameOf('trash')?.classList.contains('gone')).toBe(false);

    cleanGroup(context.send, 'trash');
    await Promise.resolve();
    expect(opacity(3)).toBe('0.8');
  });

  it('lets nothing be clicked while it runs, and says "Cleaned so far" with the size of the finished groups', async () => {
    const context = await running();

    expect(screen.queryAllByRole('checkbox')).toHaveLength(0);
    expect(within(hero()).getByText('Cleaned so far')).toBeTruthy();
    expect(within(hero()).getByText('0 B')).toBeTruthy();

    cleanGroup(context.send, 'target');
    await Promise.resolve();

    expect(within(hero()).getByText('88 GB')).toBeTruthy();
    expect(document.querySelector('.endtext')?.textContent).toContain('CLEANING');
  });

  it('shows the freed space the backend measured, and the disk after it, once the run is over', async () => {
    const context = await running();

    cleanGroup(context.send, 'target');
    cleanGroup(context.send, 'trash');
    cleanGroup(context.send, 'thin');
    finish(context.send, context.scanId, { freed: 100 * GB });
    await Promise.resolve();

    expect(within(hero()).getByText('Freed')).toBeTruthy();
    expect(within(hero()).getByText('100 GB')).toBeTruthy();
    expect(within(hero()).getByText('1.31 TB used · 69%')).toBeTruthy();
    expect(
      [...document.querySelectorAll('.endtext')].map((end) => end.textContent?.replace(/\s+/g, ' ').trim()),
    ).toEqual(['FREED 100 GB', 'STAYS ON DISK 1.31 TB']);
  });

  it('does not invent a freed size when the backend could not measure it', async () => {
    const context = await running();

    cleanGroup(context.send, 'target');
    finish(context.send, context.scanId, { freed: undefined });
    await Promise.resolve();

    expect(within(hero()).getAllByText('Not measured')).toHaveLength(2);
    expect(document.querySelector('.endtext')?.textContent).toContain('CLEANED');
  });

  it('marks a group that had a problem, and a group a cancelled run never reached goes back to its side', async () => {
    const context = await running();

    context.send({ type: 'groupStarted', ruleId: 'target', targets: 1 });
    context.send({
      type: 'targetDone',
      ruleId: 'target',
      path: '/x',
      allocated: 1,
      outcome: { kind: 'failed', detail: 'busy', problems: 0 },
    });
    context.send({ type: 'groupFinished', ruleId: 'target' });
    await Promise.resolve();

    expect(nameOf('target')?.textContent).toBe('Title of target (problem)');

    finish(context.send, context.scanId, { cancelled: true, freed: 0 });
    await Promise.resolve();

    // `trash` and `thin` were queued and never reached: they are on the stay side again.
    expect(ribbons()[3]?.getAttribute('fill-opacity')).toBe('0.16');
  });
});

const STALE = 'From the scan before the cleanup — rescan to update';
const freedBar = (): Element => document.querySelector('rect.end.freed') as Element;
const freedCaption = (): HTMLElement => document.querySelector('.cap.freed') as HTMLElement;

describe('ChartCard: "Will be freed" is green', () => {
  it('draws the bar and the caption of the freed side with the green token, not the accent', async () => {
    const context = await fixture();

    show(context);

    expect(freedCaption().textContent).toBe('WILL BE FREED');
    expect(freedBar().getAttribute('style')).toContain('var(--green)');
    expect(freedCaption().getAttribute('style')).toContain('var(--green)');
    expect(freedBar().getAttribute('style')).not.toContain('accent');
    expect(freedCaption().getAttribute('style')).not.toContain('accent');
  });

  it('keeps them green while it runs and after it, and leaves the stay bar alone', async () => {
    const context = await fixture();

    show(context);
    await context.store.cleanSelected();

    expect(freedCaption().textContent).toBe('CLEANING');
    expect(freedCaption().getAttribute('style')).toContain('var(--green)');

    cleanGroup(context.send, 'target');
    finish(context.send, context.scanId);
    await Promise.resolve();

    expect(freedCaption().textContent).toBe('FREED');
    expect(freedBar().getAttribute('style')).toContain('var(--green)');
    expect(document.querySelector('rect.end.stays')?.getAttribute('style')).toBeNull();
  });
});

describe('ChartCard after a cleanup, until a rescan', () => {
  const stale = (element: Element | null): boolean => element?.classList.contains('stale') ?? false;
  const stat = (label: string): HTMLElement => screen.getByText(label).parentElement as HTMLElement;

  it('shows the figures of the scan in full before anything ran', async () => {
    const context = await fixture();

    show(context);

    expect(stale(stat('Selected to free'))).toBe(false);
    expect(stale(stat('Disk after cleanup'))).toBe(false);
    expect(stale(screen.getByRole('group', { name: 'Cleanup chart' }))).toBe(false);
    expect(document.querySelector('.hero [title], .flow[title]')).toBeNull();
  });

  it('greys the hero figures and the chart, with a tooltip that says why, once the scan is outdated', async () => {
    const context = await fixture();

    show(context);
    context.scan.markOutdated(context.scanId);
    await Promise.resolve();

    for (const element of [
      stat('Selected to free'),
      stat('Disk after cleanup'),
      screen.getByRole('group', { name: 'Cleanup chart' }),
    ]) {
      expect(stale(element)).toBe(true);
      expect(element.getAttribute('title')).toBe(STALE);
    }

    // The words of the legend are not figures of the scan.
    expect(stale(document.querySelector('.legend'))).toBe(false);
  });

  it('greys the disk after the cleanup and the chart after a run, but not what the backend measured as freed', async () => {
    const context = await fixture();

    show(context);
    await context.store.cleanSelected();
    cleanGroup(context.send, 'target');
    finish(context.send, context.scanId, { freed: 100 * GB });
    await Promise.resolve();

    expect(stale(stat('Freed'))).toBe(false);
    expect(stale(stat('Disk after cleanup'))).toBe(true);
    expect(stale(screen.getByRole('group', { name: 'Cleanup chart' }))).toBe(true);
  });
});

describe('ChartCard with what the user marked in the Explorer', () => {
  async function marked(): Promise<Fixture> {
    const context = await fixture();

    context.store.mark({ id: 500, allocated: 40 * GB, ancestors: [1] });

    return context;
  }

  const rows = (): string[] =>
    [...document.querySelectorAll('.rows .row')].map(
      (item) => item.textContent?.replace(/\s+/g, ' ').trim() ?? '',
    );

  it('shows no such group when nothing was marked or run', async () => {
    const context = await fixture();

    show(context);

    expect(rows().some((text) => text.includes('Marked in the Explorer'))).toBe(false);
  });

  it('draws it as a group of its own in the review colour, not clickable, once its run starts', async () => {
    const context = await marked();

    show(context);
    await context.store.cleanMarked();
    await context.store.confirmReview();

    expect(rows()).toContain('Marked in the Explorer 40 GB');
    expect(document.querySelectorAll('.node.review')).toHaveLength(2);
    expect(screen.queryAllByRole('checkbox')).toHaveLength(0);
    expect(document.querySelector('.endtext')?.textContent).toContain('CLEANING 40 GB');
  });

  it('counts it in "Cleaned so far" and in what was cleaned, with the freed side showing it', async () => {
    const context = await marked();

    show(context);
    await context.store.cleanMarked();
    await context.store.confirmReview();

    expect(within(hero()).getByText('Cleaned so far')).toBeTruthy();
    expect(within(hero()).getByText('0 B')).toBeTruthy();

    cleanGroup(context.send, 'marked');
    await Promise.resolve();

    expect(within(hero()).getByText('40 GB')).toBeTruthy();

    finish(context.send, context.scanId, { freed: undefined });
    await Promise.resolve();

    // The backend could not measure what was freed: the figure is what was cleaned, from the plan.
    expect(document.querySelector('.endtext')?.textContent).toContain('CLEANED 40 GB');
  });

  it('keeps it after the run, on the freed side, until another scan is loaded', async () => {
    const context = await marked();

    show(context);
    await context.store.cleanMarked();
    await context.store.confirmReview();
    cleanGroup(context.send, 'marked');
    finish(context.send, context.scanId, { freed: 0 });
    await Promise.resolve();

    expect(rows()).toContain('Marked in the Explorer 40 GB');
    expect(within(hero()).getByText('Freed')).toBeTruthy();

    context.store.load(context.scanId + 1, DATA);
    await Promise.resolve();

    expect(rows().some((text) => text.includes('Marked in the Explorer'))).toBe(false);
  });

  it('leaves it on the stay side when a cancelled run never reached it', async () => {
    const context = await marked();

    show(context);
    await context.store.cleanMarked();
    await context.store.confirmReview();
    finish(context.send, context.scanId, { cancelled: true, freed: 0 });
    await Promise.resolve();

    expect(document.querySelector('.endtext')?.textContent).not.toContain('40 GB');
    expect(rows()).toContain('Marked in the Explorer 40 GB');
  });
});
