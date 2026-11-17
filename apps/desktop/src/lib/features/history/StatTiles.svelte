<script lang="ts">
  import type { HistoryOverviewDto } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { DAYS_FOR_GROWTH, formatDaysUntil } from './format';
  import ProgressRing from './ProgressRing.svelte';

  interface Props {
    data: HistoryOverviewDto;
  }

  let { data }: Props = $props();

  type Tile =
    | {
        kind: 'figure';
        id: string;
        /** The figure, or the words that stand in for it. */
        big: string;
        /** Set when `big` is words, not a figure: they are smaller and muted. */
        words?: boolean;
        caption: string;
      }
    | {
        kind: 'progress';
        id: string;
        /** The days recorded so far, toward the days needed. */
        days: number;
        tone: 'cyan' | 'slate';
        big: string;
        unit: string;
        caption: string;
      }
    | { kind: 'trash'; id: string; big: string; caption: string };

  /** "growth rate shows in 4 days": what is still to wait for, as a plain promise. */
  const showsIn = (what: string, left: number): string =>
    `${what} shows in ${left} ${left === 1 ? 'day' : 'days'}`;

  const tiles = $derived.by((): Tile[] => {
    const { thin, growthPerDay, daysUntilFull, freed, cleanupCount, restorable, samples } = data;

    // The backend calls a history thin until the days span a week, so the days shown never reach the full week.
    const days = Math.min(samples.length, DAYS_FOR_GROWTH - 1);
    const left = DAYS_FOR_GROWTH - days;
    const last = samples.at(-1);

    const growth: Tile = thin
      ? {
          kind: 'progress',
          id: 'growth',
          days,
          tone: 'cyan',
          big: String(days),
          unit: `of ${DAYS_FOR_GROWTH} days`,
          caption: showsIn('growth rate', left),
        }
      : growthPerDay === undefined
        ? { kind: 'figure', id: 'growth', big: 'no growth', words: true, caption: 'average growth per day' }
        : {
            kind: 'figure',
            id: 'growth',
            big: `+${fmt.size(growthPerDay)}`,
            caption: 'average growth per day',
          };

    const full: Tile = thin
      ? {
          kind: 'progress',
          id: 'full',
          days,
          tone: 'slate',
          big: last ? fmt.size(Math.max(0, last.total - last.used)) : 'First sample',
          unit: last ? 'free' : 'today',
          caption: showsIn('time until full', left),
        }
      : daysUntilFull === undefined
        ? {
            kind: 'figure',
            id: 'full',
            big: 'not at this rate',
            words: true,
            caption: 'until the disk is full',
          }
        : {
            kind: 'figure',
            id: 'full',
            big: formatDaysUntil(daysUntilFull),
            caption: 'until the disk is full at this rate',
          };

    return [
      {
        kind: 'figure',
        id: 'freed',
        big: fmt.size(freed),
        caption:
          cleanupCount === 0
            ? 'freed, no cleanup yet'
            : `freed in ${cleanupCount} ${cleanupCount === 1 ? 'cleanup' : 'cleanups'}`,
      },
      growth,
      full,
      restorable > 0
        ? {
            kind: 'figure',
            id: 'restore',
            big: fmt.size(restorable),
            caption: 'can still be restored from trash',
          }
        : { kind: 'trash', id: 'restore', big: 'Trash is clear', caption: 'nothing waiting to be restored' },
    ];
  });
</script>

<div class="stats">
  {#each tiles as tile (tile.id)}
    {#if tile.kind === 'figure'}
      <Card class="stat">
        <b class="num" class:words={tile.words}>{tile.big}</b>
        <small>{tile.caption}</small>
      </Card>
    {:else if tile.kind === 'progress'}
      <Card class="stat row">
        <ProgressRing
          value={tile.days}
          max={DAYS_FOR_GROWTH}
          tone={tile.tone}
          pulse={tile.tone === 'slate'}
          label={tile.tone === 'cyan' ? 'Days recorded' : undefined}
        />
        <div>
          <b class="num compact">{tile.big} <span>{tile.unit}</span></b>
          <small>{tile.caption}</small>
        </div>
      </Card>
    {:else}
      <Card class="stat row">
        <svg class="trash" width="46" height="46" viewBox="0 0 46 46" fill="none" aria-hidden="true">
          <!-- The can of the empty Cleanups card, at the size of the rings beside it. -->
          <path
            d="M12 15 L14.5 42 H31.5 L34 15 Z"
            fill="var(--cyan)"
            stroke="var(--cyan)"
            stroke-width="2"
            stroke-linejoin="round"
          />
          <path
            d="M18 21v15M23 21v15M28 21v15"
            stroke="#0a0e1766"
            stroke-width="2.2"
            stroke-linecap="round"
          />
          <g class="lid">
            <path d="M8.5 11h29" stroke="var(--cyan)" stroke-width="3.5" stroke-linecap="round" />
            <path d="M18.5 9.5v-3.5h9v3.5" stroke="var(--cyan)" stroke-width="2.2" stroke-linejoin="round" />
          </g>
        </svg>
        <div>
          <b class="compact">{tile.big}</b>
          <small>{tile.caption}</small>
        </div>
      </Card>
    {/if}
  {/each}
</div>

<style>
  .stats {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 16px;
  }

  .stats :global(.stat) {
    justify-content: center;
  }

  /* A tile with a picture beside its figure. */
  .stats :global(.stat.row) {
    flex-direction: row;
    justify-content: flex-start;
    align-items: center;
    gap: 14px;
  }

  b {
    display: block;
    font-size: 26px;
    line-height: 30px;
  }

  b.compact {
    font-size: 20px;
    line-height: 26px;
  }

  b.compact span {
    font-size: 13px;
    font-weight: 500;
    color: var(--muted);
  }

  /* Words in the place of a figure take the same line, smaller. */
  b.words {
    font-size: 17px;
    font-weight: 600;
    color: var(--muted);
  }

  small {
    display: block;
    color: var(--muted);
    font-size: 12px;
    line-height: 20px;
  }

  .trash {
    flex: none;
  }

  /* Hinged on its right end, like the can in the Cleanups card: the left end lifts now and then. */
  .lid {
    transform-origin: 37.5px 11px;
    animation: lid 3.2s var(--ease-out) infinite;
  }

  @keyframes lid {
    0%,
    70%,
    100% {
      transform: rotate(0);
    }

    80% {
      transform: rotate(16deg);
    }
  }

  @media (max-width: 900px) {
    .stats {
      grid-template-columns: 1fr 1fr;
    }
  }
</style>
