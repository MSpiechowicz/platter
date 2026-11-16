<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';

  interface Props extends Omit<HTMLInputAttributes, 'type' | 'checked' | 'value'> {
    checked?: boolean;
  }

  let { checked = $bindable(false), class: className = '', ...rest }: Props = $props();
</script>

<!--
  The switch is drawn with real elements, not with `::after` on the input: WebKitGTK does not draw pseudo-elements
  on a form control the way Chrome does, so the knob looked different in the app window. The input stays the
  control (it takes the click, the keyboard and the label) and lies invisibly on top of the drawing.
-->
<span class="sw {className}">
  <input type="checkbox" role="switch" bind:checked {...rest} />
  <span class="track" aria-hidden="true"><span class="knob"></span></span>
</span>

<style>
  /* 36 × 20 track, 16 px knob, 2 px inset on every side in both states. */
  .sw {
    position: relative;
    display: block;
    width: 36px;
    height: 20px;
    flex: none;
  }

  input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: pointer;
    z-index: 1;
  }

  .track {
    display: block;
    width: 100%;
    height: 100%;
    border-radius: 10px;
    background: #ffffff1c;
    transition: background 0.2s;
  }

  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--text);
    transition: transform 0.2s;
  }

  input:checked + .track {
    background: var(--accent);
  }

  /* The knob keeps its colour: a dark knob on the bright track looked smaller than the light one, though both are
     16 px. Only the track says "on". */
  input:checked + .track .knob {
    transform: translateX(16px);
  }

  input:focus-visible + .track {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  input:disabled {
    cursor: default;
  }

  input:disabled + .track {
    opacity: 0.4;
  }
</style>
