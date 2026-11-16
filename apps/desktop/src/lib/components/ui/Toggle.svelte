<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';

  interface Props extends Omit<HTMLInputAttributes, 'type' | 'checked' | 'value'> {
    checked?: boolean;
  }

  let { checked = $bindable(false), class: className = '', ...rest }: Props = $props();
</script>

<input type="checkbox" role="switch" class="sw {className}" bind:checked {...rest} />

<style>
  .sw {
    appearance: none;
    -webkit-appearance: none;
    display: block;
    width: 36px;
    height: 20px;
    border: 0;
    padding: 0;
    margin: 0;
    border-radius: 10px;
    background: #ffffff1c;
    position: relative;
    cursor: pointer;
    transition: background 0.2s;
    flex: none;
  }

  .sw::after {
    content: '';
    position: absolute;
    left: 3px;
    top: 50%;
    width: 14px;
    height: 14px;
    margin-top: -7px;
    border-radius: 50%;
    background: var(--text);
    transition: transform 0.2s;
  }

  .sw:checked {
    background: var(--accent);
  }

  .sw:checked::after {
    transform: translateX(16px);
    background: var(--on-accent);
  }

  .sw:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>
