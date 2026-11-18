import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it } from 'vitest';
import type { HistoryCleanupDto, HistoryOverviewDto, HistorySampleDto } from '../../api/types';
import { settings } from '../../state/settings.svelte';
import { tip } from '../../components/ui/tooltip.svelte';
import UsageDaysCard from './UsageDaysCard.svelte';

const GB = 1_000_000_000;

const sample = (day: string, used: number): HistorySampleDto => ({ day, used: used * GB, total: 1000 * GB });

function overview(extra: Partial<HistoryOverviewDto> = {}): HistoryOverviewDto {
  return {
    days: 182,
    today: '2026-11-18',
    samples: [
      sample('2026-11-10', 400),
      sample('2026-11-11', 410),
      sample('2026-11-12', 412),
      sample('2026-11-13', 414),
      sample('2026-11-14', 416),
      sample('2026-11-15', 418),
      sample('2026-11-16', 380),
      sample('2026-11-17', 385),
      sample('2026-11-18', 390),
    ],
    without: [],
    cleanups: [
      {
        id: 1,
        at: 0,
        day: '2026-11-16',
        what: ['Docker data'],
        freed: 40 * GB,
        methods: [{ kind: 'deleted' }],
        status: 'permanent',
        restorable: 0,
      },
    ],
    freed: 40 * GB,
    cleanupCount: 1,
    restorable: 0,
    thin: false,
    ...extra,
  };
}

/** `count` days of 1 GB growth each, the last one today (2026-11-18). */
const daily = (count: number): HistorySampleDto[] =>
  Array.from({ length: count }, (_, index) =>
    sample(new Date(Date.UTC(2026, 10, 18 - (count - 1 - index))).toISOString().slice(0, 10), 400 + index),
  );

const [docker] = overview().cleanups as [HistoryCleanupDto];

describe('UsageDaysCard', () => {
  beforeEach(() => {
    settings.set('units', 'GB');
  });

  it('draws a bar for each day that has a change, in the colour of what happened', () => {
    const { container } = render(UsageDaysCard, { props: { data: overview() } });

    const fills = [...container.querySelectorAll('rect.bar')].map((bar) => bar.getAttribute('fill'));

    // 7 rises, and the cleanup day with its two parts: 2 GB written (amber) and 40 GB freed (green).
    expect(fills).toHaveLength(9);
    expect(fills.filter((fill) => fill === 'var(--green)')).toHaveLength(1);
    expect(fills.filter((fill) => fill === 'var(--amber)')).toHaveLength(8);
    expect(screen.getByRole('img').getAttribute('aria-label')).toMatch(/last 91 days/);
  });

  it('shows the net change, what was added with its rate, and what the cleanups freed', () => {
    render(UsageDaysCard, { props: { data: overview() } });

    expect(screen.getByText('Disk emptier by').nextElementSibling?.textContent?.trim()).toBe('10 GB');
    expect(screen.getByText('Written').nextElementSibling?.textContent).toContain('30 GB');
    expect(screen.getByText('Freed by cleanups').nextElementSibling?.textContent?.trim()).toBe('40 GB');
  });

  it('writes the freed size under the day of a cleanup', () => {
    const { container } = render(UsageDaysCard, { props: { data: overview() } });

    expect([...container.querySelectorAll('text.freed')].map((text) => text.textContent)).toEqual(['40 GB']);
  });

  it('tells the day, the change, the cleanup and the used space in a tooltip on hover', async () => {
    const { container } = render(UsageDaysCard, { props: { data: overview() } });
    const hits = container.querySelectorAll('.hit');

    // Every recorded day has a target: the 16th is the third from the last.
    await fireEvent.mouseEnter(hits[hits.length - 3] as Element);

    expect(tip.content.title).toBe('16 Nov');
    expect(tip.content.items).toEqual([
      '+2.0 GB written · −40 GB freed',
      'Cleanup · Docker data',
      '380 GB used',
    ]);
  });

  it('changes the range without asking for the history again, and drops the days outside it', async () => {
    const data = overview({ samples: daily(60), cleanups: [] });
    const { container } = render(UsageDaysCard, { props: { data } });

    expect(container.querySelectorAll('rect.bar')).toHaveLength(59);

    await fireEvent.click(screen.getByRole('button', { name: '30 d' }));

    expect(screen.getByRole('img').getAttribute('aria-label')).toMatch(/last 30 days/);
    expect(container.querySelectorAll('rect.bar')).toHaveLength(30);
    expect(container.querySelectorAll('.hit')).toHaveLength(30);
  });

  it('writes the net change in amber when it is a rise, and not in green when it is a fall', () => {
    const { container, unmount } = render(UsageDaysCard, {
      props: { data: overview({ samples: daily(60), cleanups: [] }) },
    });

    expect(container.querySelector('b.net')?.classList.contains('up')).toBe(true);
    unmount();

    const fall = render(UsageDaysCard, { props: { data: overview() } });
    const net = fall.container.querySelector('b.net') as HTMLElement;

    expect(net.textContent?.trim()).toBe('10 GB');
    expect(net.classList.contains('up')).toBe(false);
    expect(net.classList.contains('down')).toBe(false);
  });

  it('lists each day that has a bar for a screen reader, with its change and cleanup', () => {
    const { container } = render(UsageDaysCard, { props: { data: overview() } });
    const lines = [...container.querySelectorAll('.sr li')].map((item) => item.textContent);

    expect(lines).toHaveLength(8);
    expect(lines).toContain('16 Nov: +2.0 GB written · −40 GB freed. Cleanup · Docker data');
    expect(lines).toContain('17 Nov: +5.0 GB');
  });

  it('draws a green fall under the amber rise on a cleanup day whose net is a rise', () => {
    const { container } = render(UsageDaysCard, {
      props: {
        data: overview({
          samples: [sample('2026-11-16', 400), sample('2026-11-17', 410)],
          cleanups: [{ ...docker, day: '2026-11-17', freed: 5 * GB }],
        }),
      },
    });

    const fills = [...container.querySelectorAll('rect.bar')].map((bar) => bar.getAttribute('fill'));

    expect(fills).toEqual(['var(--amber)', 'var(--green)']);
  });

  it('still draws a cleanup that has no sample before it', () => {
    const { container } = render(UsageDaysCard, {
      props: {
        data: overview({
          samples: [sample('2026-11-17', 400), sample('2026-11-18', 401)],
          cleanups: [{ ...docker, day: '2026-11-15', freed: 5 * GB }],
        }),
      },
    });

    const fills = [...container.querySelectorAll('rect.bar')].map((bar) => bar.getAttribute('fill'));

    expect(fills).toEqual(['var(--green)', 'var(--amber)']);
    expect([...container.querySelectorAll('text.freed')].map((text) => text.textContent)).toEqual(['5.0 GB']);
  });

  it('tells only what was freed on a cleanup day with no sample before it', async () => {
    const { container } = render(UsageDaysCard, {
      props: {
        data: overview({
          samples: [sample('2026-11-17', 400), sample('2026-11-18', 401)],
          cleanups: [{ ...docker, day: '2026-11-15', freed: 5 * GB }],
        }),
      },
    });

    await fireEvent.mouseEnter(container.querySelectorAll('.hit')[0] as Element);

    expect(tip.content.items).toEqual(['−5.0 GB freed', 'Cleanup · Docker data']);
  });

  it('says there is nothing to draw when the only cleanup freed nothing and no two days follow each other', () => {
    const { container } = render(UsageDaysCard, {
      props: {
        data: overview({
          samples: [sample('2026-11-10', 400), sample('2026-11-12', 401)],
          cleanups: [{ ...docker, day: '2026-11-11', freed: 0 }],
        }),
      },
    });

    expect(container.querySelector('svg.chart')).toBeNull();
    expect(screen.getByText(/No two days in a row/)).toBeTruthy();
  });

  it('says how long the history is instead of drawing it when it is thin', () => {
    const { container } = render(UsageDaysCard, {
      props: { data: overview({ thin: true, samples: [sample('2026-11-17', 1), sample('2026-11-18', 2)] }) },
    });

    expect(screen.getByText(/2 days so far/)).toBeTruthy();
    expect(container.querySelector('svg.chart')).toBeNull();
  });

  it('shows the problem of the history in one line instead of the plot', () => {
    const { container } = render(UsageDaysCard, {
      props: {
        data: overview({ problem: 'The history file could not be read.', samples: [], cleanups: [] }),
      },
    });

    expect(screen.getByText('The history file could not be read.')).toBeTruthy();
    expect(container.querySelector('svg.chart')).toBeNull();
  });

  it('says that the history could not be read when it failed', () => {
    render(UsageDaysCard, { props: { error: new Error('disk on fire') } });

    expect(screen.getByText(/could not be read/)).toBeTruthy();
  });
});
