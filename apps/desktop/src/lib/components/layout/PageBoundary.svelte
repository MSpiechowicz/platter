<script lang="ts">
  import type { Snippet } from 'svelte';
  import Button from '../ui/Button.svelte';
  import Card from '../ui/Card.svelte';
  import Icon from '../ui/Icon.svelte';

  interface Props {
    children: Snippet;
  }

  let { children }: Props = $props();

  // A page that fails to render must not take the sidebar and the other pages down with it.
  function report(error: unknown): void {
    console.error('A view could not be shown', error);
  }
</script>

<svelte:boundary onerror={report}>
  {@render children()}

  {#snippet failed(_error, reset)}
    <Card>
      <div class="failed" role="alert">
        <Icon name="alert" size={26} stroke={1.3} />
        <b>This view could not be shown</b>
        <p>Something went wrong while drawing it. The rest of Platter still works.</p>
        <Button size="sm" onclick={reset}>Try again</Button>
      </div>
    </Card>
  {/snippet}
</svelte:boundary>

<style>
  /* Like the other empty states: centred and muted, with the one action below. */
  .failed {
    min-height: 240px;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 8px;
    text-align: center;
    color: var(--muted);
    font-size: 12.5px;
  }

  .failed :global(svg) {
    color: var(--red);
  }

  b {
    color: var(--text);
    font-size: 14px;
  }

  p {
    max-width: 360px;
    line-height: 1.5;
    margin-bottom: 4px;
  }
</style>
