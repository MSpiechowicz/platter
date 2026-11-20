<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { LOGO_DISK_PATH, LOGO_PIECE_PATH } from '../../components/ui/Logo.svelte';
  import { FADE_MS, LAND_EASING, LAND_MS, LOGO_DRAW_MS, opening } from '../../state/opening.svelte';

  interface Props {
    /** The logo in the menu, which the opening screen's logo flies to. Null while the menu is not there. */
    target: HTMLElement | null;
  }

  let { target }: Props = $props();

  let mark: SVGSVGElement | undefined = $state();
  let flight: Animation | undefined;

  const landing = $derived(opening.phase === 'landing');

  // The flight starts once, when the landing does.
  $effect(() => {
    if (landing) {
      return untrack(fly);
    }
  });

  onDestroy(() => flight?.cancel());

  /**
   * Flies the logo onto the menu's own logo. The logo stays where it is in the document, and only its transform
   * moves, so its drawing is not restarted (moving an element in the DOM would restart it), and the rest of the
   * screen can fade around it. Without a menu logo to land on, or in a window with no layout, the landing is over at
   * once. Returns what to undo when the screen goes.
   */
  function fly(): (() => void) | undefined {
    const side = target?.closest('aside');
    const shell = side?.parentElement;

    if (!mark || !target || !side || !shell) {
      opening.finish();
      return;
    }

    const from = mark.getBoundingClientRect();
    const brand = target.getBoundingClientRect();

    if (from.width === 0 || brand.width === 0) {
      opening.finish();
      return;
    }

    // The menu slides in at the same time, so its logo is somewhere along the way right now. Where it will rest is
    // where the menu rests (the left edge of the shell) plus the logo's place inside the menu, which the slide
    // does not change.
    const resting = shell.getBoundingClientRect().left + (brand.left - side.getBoundingClientRect().left);
    const dx = resting - from.left;
    const dy = brand.top - from.top;
    const scale = brand.width / from.width;

    // A flight that never reports its end (a hidden window) must not leave the app behind the screen.
    const timer = setTimeout(() => opening.finish(), LAND_MS + 300);

    try {
      flight = mark.animate(
        [{ transform: 'none' }, { transform: `translate(${dx}px, ${dy}px) scale(${scale})` }],
        { duration: LAND_MS, easing: LAND_EASING, fill: 'forwards' },
      );
      flight.onfinish = () => opening.finish();
    } catch {
      // No flight (an engine without the Web Animations API): the app opens at once.
      opening.finish();
    }

    return () => clearTimeout(timer);
  }

  /** What a screen reader says: the step in progress, or what the last one found once all are done. */
  const announcement = $derived.by(() => {
    const running = opening.steps.find((step) => step.status === 'running');

    if (running) {
      return `${running.label}…`;
    }

    const done = opening.steps.filter((step) => step.status === 'done').at(-1);

    return done ? [done.label, done.note].filter(Boolean).join(': ') : '';
  });

  function statusText(status: string): string {
    return status === 'running' ? 'in progress' : status;
  }
</script>

<section
  class="screen"
  class:landing
  aria-label="Opening Platter"
  style="--land-ms:{LAND_MS}ms;--fade-ms:{FADE_MS}ms;--draw-ms:{LOGO_DRAW_MS}ms"
>
  <div class="backdrop"></div>

  <div class="content">
    <!-- Drawn once: the outline, then the disk, then the piece pops out. It stays whole while the steps go on. -->
    <svg class="mark" bind:this={mark} viewBox="0 0 48 48" aria-hidden="true">
      <path class="disk" d={LOGO_DISK_PATH} />
      <path class="outline" d={LOGO_DISK_PATH} />
      <path class="piece" d={LOGO_PIECE_PATH} />
    </svg>

    <div class="text">
      <h2>Opening Platter</h2>
      <p class="sub">Getting your disks and your last scan ready</p>

      <p class="vh" role="status" aria-live="polite">{announcement}</p>

      <ul class="steps">
        {#each opening.steps as step (step.id)}
          <li class={step.status} class:warn={step.warn}>
            <span class="dot" aria-hidden="true"></span>
            <span class="label">{step.label}</span>
            <span class="vh">{statusText(step.status)}</span>
            {#if step.note}<small>{step.note}</small>{/if}
          </li>
        {/each}
      </ul>
    </div>
  </div>
</section>

<style>
  /*
   * As wide as the window, scrollbar included: the page laid out behind it can grow a scrollbar while the steps run
   * (a saved scan opens its Overview), and centring on what is left would jump the logo and the steps sideways.
   */
  .screen {
    position: fixed;
    top: 0;
    bottom: 0;
    left: 0;
    width: 100vw;
    z-index: 50;
    display: grid;
    place-items: center;
  }

  /* The screen's own colour fades on its own, so the logo can go on flying over what shows through. */
  .backdrop {
    position: absolute;
    inset: 0;
    background: var(--bg);
  }

  .content {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  .landing {
    pointer-events: none;
  }

  .landing .backdrop,
  .landing .text {
    opacity: 0;
    transition: opacity var(--fade-ms) ease;
  }

  .text {
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  h2 {
    font-size: 20px;
    font-weight: 600;
    margin: 22px 0 4px;
  }

  .sub {
    color: var(--muted);
    margin: 0;
    font-size: 13px;
  }

  .mark {
    width: 120px;
    height: 120px;
    flex: none;
    overflow: visible;
    transform-origin: 0 0;
  }

  /* Whole at rest (reduced motion plays none of it): the disk filled, no outline, the piece out of the disk. */
  .disk {
    fill: var(--logo-ring);
    animation: fill var(--draw-ms) ease-in-out both;
  }

  .outline {
    fill: none;
    stroke: var(--logo-ring);
    stroke-width: 1.2;
    stroke-dasharray: 160;
    opacity: 0;
    animation: line var(--draw-ms) ease-in-out both;
  }

  .piece {
    fill: var(--accent);
    transform-box: fill-box;
    transform-origin: center;
    transform: translate(3.4px, -3.4px);
    animation: pop var(--draw-ms) cubic-bezier(0.3, 1.5, 0.5, 1) both;
  }

  @keyframes line {
    0% {
      stroke-dashoffset: 160;
      opacity: 1;
    }
    55% {
      stroke-dashoffset: 0;
      opacity: 1;
    }
    80%,
    100% {
      stroke-dashoffset: 0;
      opacity: 0;
    }
  }

  @keyframes fill {
    0%,
    48% {
      fill-opacity: 0;
    }
    78%,
    100% {
      fill-opacity: 1;
    }
  }

  @keyframes pop {
    0%,
    70% {
      transform: translate(3.4px, -3.4px) scale(0);
      opacity: 0;
    }
    92%,
    100% {
      transform: translate(3.4px, -3.4px) scale(1);
      opacity: 1;
    }
  }

  .steps {
    list-style: none;
    padding: 0;
    margin: 26px 0 0;
    width: 440px;
    max-width: calc(100vw - 48px);
    display: flex;
    flex-direction: column;
    gap: 12px;
    font-size: 14px;
  }

  .steps li {
    display: flex;
    align-items: center;
    gap: 12px;
    color: var(--dim);
    opacity: 0.45;
    transition:
      color 0.3s,
      opacity 0.3s;
  }

  .dot {
    width: 18px;
    height: 18px;
    flex: none;
    box-sizing: border-box;
    border-radius: 50%;
    border: 1.5px solid var(--line);
    display: grid;
    place-items: center;
    font-size: 11px;
    transition:
      border-color 0.3s,
      background 0.3s;
  }

  .steps li.running {
    color: var(--text);
    opacity: 1;
  }

  .running .dot {
    border-color: var(--accent);
    border-top-color: transparent;
    animation: spin 0.8s linear infinite;
  }

  .steps li.done {
    color: var(--muted);
    opacity: 1;
  }

  .done .dot {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--on-accent);
  }

  .done .dot::after {
    content: '✓';
    font-weight: 800;
  }

  /* Skipped: nothing to check, so it stays dim with no spinner and no tick. */
  .steps li.skipped {
    opacity: 0.3;
  }

  .label,
  small {
    white-space: nowrap;
  }

  small {
    margin-left: auto;
    color: var(--dim);
    font-family: var(--mono);
    font-size: 12px;
  }

  .warn small {
    color: var(--amber);
  }

  .vh {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
