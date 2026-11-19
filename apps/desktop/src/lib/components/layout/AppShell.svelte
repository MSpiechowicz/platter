<script lang="ts">
  import type { Snippet } from 'svelte';
  import { LAND_EASING, LAND_MS, type OpeningPhase } from '../../state/opening.svelte';

  interface Props {
    /** The sidebar shows as a 60 px icon rail. */
    collapsed: boolean;
    /**
     * Where the app is on its way out of the opening screen. While `opening` it is laid out but hidden and out of
     * reach, so the menu's logo can be measured. While `landing` the menu slides in and the page fades in.
     */
    phase?: OpeningPhase;
    sidebar: Snippet;
    children: Snippet;
  }

  let { collapsed, phase = 'done', sidebar, children }: Props = $props();

  let main: HTMLElement | undefined = $state();
  // Set by the effect below, which runs on the first paint too.
  let wasOpening = false;

  // When the opening screen goes, the keyboard starts on the page, not on nothing.
  $effect(() => {
    const done = phase === 'done';

    // Only when nothing else has the keyboard: a dialog that opened meanwhile keeps it.
    if (done && wasOpening && (document.activeElement === null || document.activeElement === document.body)) {
      main?.focus({ preventScroll: true });
    }

    wasOpening = !done;
  });

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

<div
  class="app"
  class:mini={collapsed}
  class:anim={animate}
  class:opening={phase === 'opening'}
  class:landing={phase === 'landing'}
  inert={phase !== 'done'}
  style="--land-ms:{LAND_MS}ms;--land-ease:{LAND_EASING}"
>
  {@render sidebar()}
  <main class="main" tabindex="-1" bind:this={main}>{@render children()}</main>
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

  /* Hidden but laid out, so the menu's logo has its place to be measured. */
  .app.opening {
    visibility: hidden;
  }

  .landing :global(.side) {
    animation: slide-in var(--land-ms) var(--land-ease);
  }

  .landing .main {
    animation: page-in 0.4s ease 0.2s both;
  }

  @keyframes slide-in {
    from {
      transform: translateX(-100%);
    }
  }

  @keyframes page-in {
    from {
      opacity: 0;
    }
  }

  .main {
    outline: none;
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
