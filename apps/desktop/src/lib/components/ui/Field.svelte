<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    label: string;
    description?: string;
    /** The control on the right. It gets the ids of the label and the description for `aria-labelledby` / `aria-describedby`. */
    control: Snippet<[labelId: string, descriptionId: string]>;
  }

  let { label, description, control }: Props = $props();

  const id = $props.id();
</script>

<div class="field">
  <div class="text">
    <span id="{id}-label">{label}</span>
    {#if description}<small id="{id}-desc">{description}</small>{/if}
  </div>
  {@render control(`${id}-label`, `${id}-desc`)}
</div>

<style>
  .field {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 11px 0;
    border-top: 1px solid var(--line);
  }

  .field:first-of-type {
    border-top: 0;
    padding-top: 0;
  }

  .text {
    flex: 1;
    min-width: 0;
  }

  small {
    display: block;
    color: var(--muted);
    font-size: 12px;
    margin-top: 1px;
  }
</style>
