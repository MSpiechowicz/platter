<script lang="ts" generics="T extends string">
  interface Option {
    value: T;
    label: string;
  }

  interface Props {
    options: readonly Option[];
    value: T;
    /** Accessible name of the group. */
    label: string;
    onchange?: (value: T) => void;
  }

  let { options, value = $bindable(), label, onchange }: Props = $props();

  function choose(next: T): void {
    if (next === value) {
      return;
    }

    value = next;
    onchange?.(next);
  }
</script>

<div class="seg" role="group" aria-label={label}>
  {#each options as option (option.value)}
    <button
      type="button"
      class:on={option.value === value}
      aria-pressed={option.value === value}
      onclick={() => choose(option.value)}
    >
      {option.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: flex;
    border: 1px solid var(--line);
    border-radius: 7px;
    overflow: hidden;
  }

  button {
    padding: 6px 12px;
    font-size: 12.5px;
    color: var(--muted);
    transition: color 0.15s;
  }

  button + button {
    border-left: 1px solid var(--line);
  }

  button:hover {
    color: var(--text);
  }

  .on {
    background: #ffffff14;
    color: var(--text);
  }

  button:focus-visible {
    outline-offset: -2px;
  }
</style>
