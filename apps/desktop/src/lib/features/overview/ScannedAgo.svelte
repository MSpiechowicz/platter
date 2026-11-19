<script lang="ts">
  import { formatAgo } from '../../format';

  interface Props {
    /** When the scan finished, in unix seconds. */
    at: number;
  }

  let { at }: Props = $props();

  /** The words change by the minute, so it looks again once a minute. */
  const REFRESH_MS = 60_000;
  const OLD_AFTER_MS = 24 * 60 * 60 * 1000;

  let now = $state(Date.now());

  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), REFRESH_MS);

    return () => clearInterval(timer);
  });

  const age = $derived(now - at * 1000);
  const text = $derived(formatAgo(age));
  /** A scan a day old or more may well be out of date: it says so in amber. */
  const old = $derived(age >= OLD_AFTER_MS);
</script>

<span class="scanned" class:old>Scanned {text}</span>

<style>
  .scanned {
    color: var(--muted);
  }

  .old {
    color: var(--amber);
  }
</style>
