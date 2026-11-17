<script lang="ts">
  interface Props {
    /** What is happening, in a few words: "Reading the scan…". */
    label?: string;
  }

  let { label }: Props = $props();
</script>

<!-- Fills its positioned parent. Use it through <Loading>, which applies the 300 ms rule. -->
<div class="spin" role="status">
  <svg viewBox="0 0 48 48" aria-hidden="true">
    <circle cx="24" cy="24" r="13.5" fill="none" stroke="var(--ring)" stroke-width="13" />
    <g
      ><path
        d="M32.77,6.02A20,20 0 0 1 40.96,13.40L29.94,20.29A7,7 0 0 0 27.07,17.71Z"
        fill="var(--accent)"
      /></g
    >
  </svg>
  {#if label}<span>{label}</span>{/if}
</div>

<style>
  .spin {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 12px;
    font-size: 12.5px;
    color: var(--muted);
  }

  svg {
    width: 52px;
    height: 52px;
  }

  g {
    transform-origin: 24px 24px;
    animation: spinrot 1.1s cubic-bezier(0.5, 0.15, 0.5, 0.85) infinite;
  }

  @keyframes spinrot {
    to {
      transform: rotate(360deg);
    }
  }
</style>
