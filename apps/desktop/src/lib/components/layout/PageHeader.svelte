<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    sub?: string;
    /** Right after the title. */
    lead?: Snippet;
    /** Controls on the right. Size them to the card below. */
    children?: Snippet;
  }

  let { title, sub, lead, children }: Props = $props();
</script>

<header class="top">
  <div>
    <h1>{title}</h1>
    {#if sub}<div class="sub">{sub}</div>{/if}
  </div>
  {@render lead?.()}
  <div class="sp"></div>
  {@render children?.()}
</header>

<style>
  .top {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: 18px;
    flex-wrap: wrap;
  }

  h1 {
    font-family: var(--mono);
    font-size: 17px;
    letter-spacing: 0.1em;
    font-weight: 700;
  }

  .sub {
    color: var(--muted);
    font-size: 12px;
    font-family: var(--mono);
  }

  .sp {
    flex: 1;
  }
</style>
