<script lang="ts">
  import { cleanup } from '../../state/cleanup.svelte';
  import { history } from '../../state/history.svelte';
  import { scan } from '../../state/scan.svelte';
  import Button from '../ui/Button.svelte';
  import Icon from '../ui/Icon.svelte';

  // A cleanup, an undo or a restore from History is moving files: a scan now would see them half moved.
  const blocked = $derived(cleanup.busy || cleanup.undoing || history.restoring !== undefined);

  /** The height the banner takes from the page, margin included, as a custom property of the main area. */
  const SPACE = '--banner-space';

  let banner: HTMLElement | undefined = $state();

  const titleId = $props.id();

  /*
   * What a screen reader says when the banner comes. A live region that arrives with its words already in it is
   * often not read, so it starts empty and gets them once it is on the page.
   */
  const SAID = 'This scan is out of date. Cleaning is paused until you rescan.';

  let said = $state('');

  $effect(() => {
    const timer = setTimeout(() => (said = SAID), 100);

    return () => clearTimeout(timer);
  });

  // Pages that fill the window (the Advisor, the Explorer) take this space off their height, so the banner does not
  // make them scroll. It follows the banner as it wraps in a narrow window.
  $effect(() => {
    const element = banner;
    const main = element?.closest('main');

    if (!element || !main) {
      return;
    }

    const publish = (): void => {
      const margin = parseFloat(getComputedStyle(element).marginBottom) || 0;

      main.style.setProperty(SPACE, `${element.offsetHeight + margin}px`);
    };

    publish();

    const observer = typeof ResizeObserver === 'undefined' ? undefined : new ResizeObserver(publish);

    observer?.observe(element);

    return () => {
      observer?.disconnect();
      main.style.removeProperty(SPACE);
    };
  });
</script>

<section class="banner" aria-labelledby={titleId} bind:this={banner}>
  <span class="mark"><Icon name="refresh" size={16} stroke={1.8} /></span>

  <p class="vh" role="status">{said}</p>

  <div class="words">
    <b id={titleId}>This scan is out of date</b>
    <p>The disk changed after the cleanup. Figures are from before it and cleaning is paused.</p>
  </div>

  <Button variant="primary" disabled={blocked} onclick={() => void scan.rescan()}>Rescan</Button>
</section>

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 16px;
    padding: 10px 14px;
    border: 1px solid color-mix(in srgb, var(--amber) 40%, transparent);
    border-radius: 12px;
    background: color-mix(in srgb, var(--amber) 12%, transparent);
  }

  .mark {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--amber);
    color: var(--on-accent);
  }

  .words {
    flex: 1;
    min-width: 0;
  }

  b {
    color: var(--text);
  }

  p {
    margin-top: 2px;
    font-size: 13px;
    color: var(--muted);
  }

  .vh {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
