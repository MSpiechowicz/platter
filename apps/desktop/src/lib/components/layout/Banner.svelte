<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from '../ui/Icon.svelte';
  import { publishBannerSpace } from './bannerSpace';

  interface Props {
    icon: IconName;
    title: string;
    /** What a screen reader says when the banner comes. */
    said: string;
    /** The sentence under the title. */
    children: Snippet;
    /** The buttons at the end. */
    actions: Snippet;
  }

  let { icon, title, said, children, actions }: Props = $props();

  let banner: HTMLElement | undefined = $state();

  const titleId = $props.id();

  /*
   * A live region that arrives with its words already in it is often not read, so it starts empty and gets them once
   * it is on the page.
   */
  let spoken = $state('');

  $effect(() => {
    const words = said;
    const timer = setTimeout(() => (spoken = words), 100);

    return () => clearTimeout(timer);
  });

  // Pages that fill the window (the Advisor, the Explorer) take this banner's room off their height.
  $effect(() => (banner ? publishBannerSpace(banner) : undefined));
</script>

<section class="banner" aria-labelledby={titleId} bind:this={banner}>
  <span class="mark"><Icon name={icon} size={16} stroke={1.8} /></span>

  <p class="vh" role="status">{spoken}</p>

  <div class="words">
    <b id={titleId}>{title}</b>
    <p>{@render children()}</p>
  </div>

  <div class="actions">{@render actions()}</div>
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

  .actions {
    flex: none;
    display: flex;
    gap: 8px;
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
