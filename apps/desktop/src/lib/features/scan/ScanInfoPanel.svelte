<script lang="ts">
  import { scan } from '../../state/scan.svelte';
  import { settings } from '../../state/settings.svelte';
  import Card from '../../components/ui/Card.svelte';
  import { describeTarget } from './target';

  interface Props {
    /** The path that will be or is being scanned. */
    root: string | null;
  }

  let { root }: Props = $props();

  const facts = $derived(describeTarget(scan.volumes, root, settings.units));
</script>

<Card title="What happens">
  <p>
    Platter reads folder names, sizes and dates. Nothing is changed and nothing leaves this machine. The
    result is kept in memory and dropped when you scan again or close the app.
  </p>

  {#if facts.length > 0}
    <dl>
      {#each facts as fact (fact.label)}
        <div>
          <dt>{fact.label}</dt>
          <dd class:mono={fact.label === 'Folder'} title={fact.value}>{fact.value}</dd>
        </div>
      {/each}
    </dl>
  {/if}
</Card>

<style>
  p {
    font-size: 13px;
    color: var(--muted);
  }

  dl {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
    margin-top: 18px;
  }

  div {
    min-width: 0;
  }

  dt {
    font-size: 11px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--muted);
  }

  dd {
    font-size: 18px;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  dd.mono {
    font-size: 13px;
    font-weight: 600;
    line-height: 25px;
  }
</style>
