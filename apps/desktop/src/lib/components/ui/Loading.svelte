<script lang="ts">
  import { delayed } from '../../state/delayed.svelte';
  import Spinner from './Spinner.svelte';

  interface Props {
    /** True while the parent card waits for data. */
    when: boolean;
    label: string;
  }

  let { when, label }: Props = $props();

  // Under 300 ms nothing is shown, so fast loads do not flicker.
  const visible = delayed(() => when);
</script>

{#if visible.value}
  <Spinner {label} />
{/if}
