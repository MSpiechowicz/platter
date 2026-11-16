<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';

  interface Props extends Omit<HTMLInputAttributes, 'type' | 'checked' | 'value'> {
    checked?: boolean;
  }

  let { checked = $bindable(false), class: className = '', ...rest }: Props = $props();
</script>

<!--
  The box is the input itself (drawn here, so it has one size and one look in every web view); the tick is an SVG
  stroke over it, which stays crisp in WebKitGTK, where a rotated CSS border renders thick and blurry.
-->
<span class="chk-wrap {className}">
  <input type="checkbox" class="chk" bind:checked {...rest} />
  <svg class="tick" viewBox="0 0 14 14" aria-hidden="true" focusable="false">
    <path d="M3.5 7.2 6 9.6 10.6 4.6" />
  </svg>
</span>

<style>
  .chk-wrap {
    position: relative;
    display: block;
    flex: none;
    width: 14px;
    height: 14px;
  }

  .chk {
    appearance: none;
    -webkit-appearance: none;
    display: block;
    position: relative;
    width: 14px;
    height: 14px;
    margin: 0;
    padding: 0;
    border: 1px solid var(--dim);
    border-radius: 3px;
    background: transparent;
    cursor: pointer;
    transition:
      background 0.15s,
      border-color 0.15s;
  }

  /* The click target is larger than the box; it never takes room. */
  .chk::before {
    content: '';
    position: absolute;
    inset: -6px;
  }

  .tick {
    position: absolute;
    inset: 0;
    width: 14px;
    height: 14px;
    pointer-events: none;
    fill: none;
    stroke: var(--on-accent);
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0;
  }

  .chk:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .chk:checked {
    background: var(--accent);
    border-color: var(--accent);
  }

  .chk:checked + .tick {
    opacity: 1;
  }

  .chk:disabled,
  .chk:disabled + .tick {
    opacity: 0.4;
    cursor: default;
  }

  .chk:disabled:not(:checked) + .tick {
    opacity: 0;
  }
</style>
