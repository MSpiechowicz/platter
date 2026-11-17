<script lang="ts">
  import Bar from '../../components/ui/Bar.svelte';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Icon, { type IconName } from '../../components/ui/Icon.svelte';
  import Notice from '../../components/ui/Notice.svelte';
  import type { AdvisorStatusDto } from '../../api/types';
  import type { AdvisorStore } from '../../state/advisor.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { motion } from '../../state/motion.svelte';
  import { ui } from '../../state/ui.svelte';
  import { visibleText } from '../overview/display';
  import { downloadLabel, downloadShare, receivedText } from './model';

  interface Props {
    /** The model is not on disk: not downloaded, or downloading. */
    status: AdvisorStatusDto;
    store: AdvisorStore;
  }

  let { status, store }: Props = $props();

  const titleId = $props.id();

  const downloading = $derived(status.state === 'downloading');
  const problem = $derived(store.failure ?? status.problem);
  /** A download in progress keeps its Cancel button: the slot has no room for a problem next to it. */
  const failed = $derived(!downloading && problem !== undefined);
  const share = $derived(status.download ? downloadShare(status.download) : 0);
  const percent = $derived(Math.round(share * 100));

  const reduced = $derived(motion.reduced);
  const moving = $derived(!reduced && !failed);

  // The disk stack, bottom layer first: the y of its top face. One side's height apart, so each layer hides the one
  // below and only one seam shows between them. The download's progress is the badge's ring alone.
  const LAYERS = [148, 120, 92];

  // Circumference of the badge's progress ring (r = 15).
  const RING = 2 * Math.PI * 15;

  const facts: { icon: IconName; title: string; text: string }[] = $derived([
    { icon: 'offline', title: 'Offline', text: 'Runs on this machine' },
    {
      icon: 'download',
      title: `${fmt.size(status.model.bytes)}, once`,
      text: visibleText(status.model.name),
    },
    { icon: 'lock', title: 'Your files stay here', text: 'Nothing is uploaded' },
  ]);
</script>

<Card grow aria-labelledby={titleId}>
  <div class="hero">
    <svg
      class="illu"
      class:enter={!reduced}
      class:run={moving}
      class:fast={moving && downloading}
      viewBox="0 0 240 206"
      role="img"
      aria-label="A stack of disks with a model chip above it"
    >
      {#each LAYERS as y, layer (y)}
        <g class="disk" style="--i:{layer}">
          <path class="side" d="M42 {y}v28a78 22 0 0 0 156 0v-28" />
          <ellipse class="top" class:lid={layer === LAYERS.length - 1} cx="120" cy={y} rx="78" ry="22" />
          <ellipse class="core" cx="120" cy={y} rx="34" ry="9" />
          <!--
            A slot and a light on the middle line of the layer's band. The band curves with the disk, so both sit on the
            ellipse half way between the seams (centre y + 14, radii 78 × 22): the slot follows it, the light is on it.
          -->
          <path class="slot" d="M62 {y + 28.7}A78 22 0 0 0 100 {y + 35.3}" />
          <circle class="led" cx="168" cy={y + 31.3} r="3.4" style="--d:{layer * 0.8}s" />
        </g>
      {/each}

      <g class="late">
        <path class="flow" d="M108 60v14M120 60v18M132 60v14" />
      </g>

      <g class="chip-enter">
        <g class="chip">
          <path
            class="pins"
            d="M108 8v-5M120 8v-5M132 8v-5M108 56v5M120 56v5M132 56v5M96 20h-5M96 32h-5M96 44h-5M144 20h5M144 32h5M144 44h5"
          />
          <rect class="body" x="96" y="8" width="48" height="48" rx="10" />
          <rect class="die" x="109" y="21" width="22" height="22" rx="5" />
        </g>
      </g>

      <g class="late">
        <g transform="translate(196 62)">
          {#if failed}
            <circle class="badge-warn" r="17" />
            <path class="mark" d="M0 -6.5v7.5M0 6v.2" />
          {:else if downloading}
            <circle class="badge-face" r="17" />
            <circle class="ring-track" r="15" />
            <circle
              class="ring"
              r="15"
              transform="rotate(-90)"
              style="stroke-dasharray:{share * RING} {RING}"
            />
            <g class="arrow" transform="translate(-8 -8)">
              <path d="M8 3.5v6M5.6 7.4L8 9.8l2.4-2.4M4.5 12.2h7" />
            </g>
          {:else}
            <circle class="ping" r="17" />
            <g class="pulse">
              <circle class="badge-core" r="17" />
              <g class="arrow dark" transform="translate(-10 -10) scale(1.25)">
                <path d="M8 2.5v7.5M4.8 7L8 10.2 11.2 7M3 13h10" />
              </g>
            </g>
          {/if}
        </g>
      </g>
    </svg>

    <div class="text" class:enter-text={!reduced}>
      <h2 id={titleId}>
        {downloading
          ? 'Downloading the local model for the Advisor'
          : 'Download the local model to use the Advisor'}
      </h2>

      <p class="desc">
        The Advisor answers questions about your scan in your own words, like which folder takes the most
        space or whether the docker images are safe to delete. It also labels folders that no rule covers. It
        runs offline on this machine and needs about 4.8 GB of free RAM while in use.
      </p>

      <ul class="facts">
        {#each facts as fact (fact.title)}
          <li class="fact">
            <Icon name={fact.icon} />
            <div><b>{fact.title}</b><small>{fact.text}</small></div>
          </li>
        {/each}
      </ul>

      <div class="act">
        {#if downloading}
          <div class="dl">
            {#if status.download}
              <Bar value={share} label="Download progress" />
              <div class="prow">
                <span class="num">{receivedText(status.download, fmt.size)} · {percent}%</span>
                <Button onclick={() => store.cancelDownload()}>Cancel</Button>
              </div>
            {:else}
              <Bar indeterminate label="Download progress" />
              <div class="prow">
                <span>Starting the download…</span>
                <Button onclick={() => store.cancelDownload()}>Cancel</Button>
              </div>
            {/if}
          </div>
        {:else}
          {#if problem}
            <Notice kind="error">{visibleText(problem)}</Notice>
          {/if}

          <Button variant="primary" disabled={store.asking} onclick={() => store.download()}>
            {downloadLabel(status, fmt.size)}
          </Button>

          <button class="link" type="button" onclick={() => ui.go('settings')}>Open Settings</button>
        {/if}
      </div>

      <p class="priv">
        The download is Platter's only network call. Your files never leave this machine. Verdicts always come
        from Platter's rules and safety guard. The model only reads your question.
      </p>
    </div>
  </div>
</Card>

<style>
  .hero {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 32px 24px;
    min-height: 520px;
  }

  .text {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-width: 0;
    max-width: 100%;
  }

  h2 {
    font-size: 24px;
    line-height: 1.25;
    font-weight: 700;
    max-width: 560px;
  }

  .desc {
    margin-top: 12px;
    max-width: 540px;
    color: var(--muted);
    font-size: 14px;
    line-height: 1.6;
  }

  .facts {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 10px;
    margin: 26px 0;
    padding: 0;
    list-style: none;
  }

  .fact {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 14px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--card2);
    text-align: left;
  }

  .fact :global(svg) {
    color: var(--accent);
  }

  .fact b {
    display: block;
    font-size: 13px;
    font-weight: 600;
    line-height: 1.2;
  }

  .fact small {
    display: block;
    margin-top: 2px;
    font-size: 11.5px;
    line-height: 1.3;
    color: var(--muted);
  }

  .act {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    max-width: 100%;
  }

  .act :global(.notice) {
    max-width: 440px;
    text-align: left;
  }

  .link {
    font-size: 12px;
    color: var(--accent);
  }

  .link:hover {
    color: var(--accent-hover);
  }

  .dl {
    width: 360px;
    max-width: 100%;
  }

  .prow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-top: 8px;
    height: 30px;
    font-size: 12px;
    color: var(--muted);
    text-align: left;
  }

  .priv {
    margin-top: 20px;
    max-width: 480px;
    font-size: 12px;
    line-height: 1.55;
    color: var(--dim);
  }

  /* The illustration: flat fills and strokes, the accent as the only colour. */
  .illu {
    width: 240px;
    height: auto;
    margin-bottom: 22px;
    color: var(--accent);
    --rim: #2f3b52;
    --face: #1a2335;
  }

  .side {
    fill: var(--card2);
    stroke: var(--rim);
    stroke-width: 1.5;
  }

  .top {
    fill: var(--face);
    stroke: var(--rim);
    stroke-width: 1.5;
  }

  /* The top face carries the model's place. */
  .top.lid {
    stroke: var(--accent);
    stroke-opacity: 0.55;
  }

  .core {
    fill: var(--accent);
    fill-opacity: 0.14;
  }

  .slot {
    fill: none;
    stroke: var(--rim);
    stroke-width: 2;
    stroke-linecap: round;
  }

  .led {
    fill: var(--dim);
  }

  .flow {
    fill: none;
    stroke: var(--accent);
    stroke-opacity: 0.5;
    stroke-width: 1.5;
    stroke-dasharray: 2 4;
    stroke-linecap: round;
  }

  .pins {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2;
    stroke-linecap: round;
  }

  .body {
    fill: var(--card);
    stroke: var(--accent);
    stroke-width: 2;
  }

  .die {
    fill: var(--accent);
    fill-opacity: 0.22;
    stroke: var(--accent);
    stroke-width: 2;
  }

  .badge-core {
    fill: var(--accent);
  }

  .badge-face {
    fill: var(--card);
  }

  .badge-warn {
    fill: var(--amber);
  }

  .mark {
    fill: none;
    stroke: var(--on-accent);
    stroke-width: 2.4;
    stroke-linecap: round;
  }

  .ring-track {
    fill: none;
    stroke: var(--line);
    stroke-width: 3;
  }

  .ring {
    fill: none;
    stroke: var(--accent);
    stroke-width: 3;
    stroke-linecap: round;
    transition: stroke-dasharray 1.1s var(--ease-out);
  }

  .arrow {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .arrow.dark {
    stroke: var(--on-accent);
    stroke-width: 1.7;
  }

  .ping {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.5;
    opacity: 0;
  }

  .pulse,
  .ping,
  .chip {
    transform-box: fill-box;
    transform-origin: center;
  }

  /* First appearance, once: the layers stack in from the bottom, the chip drops, then the text fades up. */
  .enter .disk {
    animation: stack-in 0.55s var(--ease-out) both;
    animation-delay: calc(var(--i) * 150ms);
  }

  .enter .chip-enter {
    animation: drop-in 0.6s var(--ease-out) 0.5s both;
  }

  .enter .late {
    animation: fade-in 0.5s ease 0.9s both;
  }

  .enter-text {
    animation: fade-up 0.6s var(--ease-out) 0.8s both;
  }

  /* Looping, slow and subtle; only while there is nothing wrong and motion is allowed. */
  .run .chip {
    animation: float 4s ease-in-out infinite;
  }

  .run .flow {
    animation: flow 1.6s linear infinite;
  }

  .run.fast .flow {
    animation-duration: 0.55s;
  }

  .run .led {
    animation: blink 2.4s ease-in-out infinite;
    animation-delay: var(--d);
  }

  .run .pulse {
    animation: pulse 2.5s ease-in-out infinite;
  }

  .run .ping {
    animation: ping 2.5s ease-out infinite;
  }

  @keyframes stack-in {
    from {
      opacity: 0;
      transform: translateY(-14px);
    }
  }

  @keyframes drop-in {
    from {
      opacity: 0;
      transform: translateY(-24px);
    }
  }

  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }

  @keyframes fade-up {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }

  @keyframes float {
    50% {
      transform: translateY(-3px);
    }
  }

  @keyframes flow {
    to {
      stroke-dashoffset: -6;
    }
  }

  @keyframes blink {
    50% {
      opacity: 0.3;
    }
  }

  @keyframes pulse {
    50% {
      transform: scale(1.06);
    }
  }

  @keyframes ping {
    from {
      opacity: 0.5;
      transform: scale(1);
    }

    to {
      opacity: 0;
      transform: scale(1.7);
    }
  }

  @media (max-width: 800px) {
    .hero {
      padding: 24px 8px;
      min-height: 0;
    }

    h2 {
      font-size: 21px;
    }

    .facts {
      flex-direction: column;
      align-items: stretch;
      width: 100%;
      max-width: 360px;
    }
  }
</style>
