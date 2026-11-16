<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /** The sidebar shows as a 60 px icon rail. */
    collapsed: boolean;
    sidebar: Snippet;
    children: Snippet;
  }

  let { collapsed, sidebar, children }: Props = $props();

  // The width animates when the user toggles the rail, but not on the first paint.
  let animate = $state(false);
  let first = true;

  $effect(() => {
    void collapsed;

    if (first) {
      first = false;
      return;
    }

    animate = true;
  });
</script>

<div class="app" class:mini={collapsed} class:anim={animate}>
  {@render sidebar()}
  <main class="main">{@render children()}</main>
</div>

<style>
  .app {
    display: grid;
    grid-template-columns: 220px 1fr;
    min-height: 100vh;
  }

  .app.anim {
    transition: grid-template-columns 0.25s cubic-bezier(0.3, 0.8, 0.3, 1);
  }

  .app.mini {
    grid-template-columns: 60px 1fr;
  }

  .main {
    padding: 20px 24px;
    min-width: 0;
    max-width: 1640px;
    width: 100%;
    margin: 0 auto;
  }

  @media (max-width: 900px) {
    .main {
      padding: 16px;
    }
  }
</style>
