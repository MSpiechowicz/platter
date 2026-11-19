<script lang="ts">
  import { formatAgo } from '../../format';

  interface Props {
    /** When the scan finished, in unix seconds. */
    at: number;
  }

  let { at }: Props = $props();

  /** The words change by the minute, so it looks again once a minute. */
  const REFRESH_MS = 60_000;

  let now = $state(Date.now());

  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), REFRESH_MS);

    return () => clearInterval(timer);
  });

  const text = $derived(formatAgo(now - at * 1000));
</script>

<p class="scanned">Scanned {text}</p>

<style>
  .scanned {
    margin: 6px 0 0;
    color: var(--muted);
    font-size: 12px;
    line-height: 1.4;
  }
</style>
