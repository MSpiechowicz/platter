<script lang="ts">
  import Button from '../../components/ui/Button.svelte';

  interface Props {
    onretry: () => void;
  }

  let { onretry }: Props = $props();
</script>

<div class="retry" role="alert">
  <p>The suggestions could not be loaded.</p>
  <Button size="sm" onclick={onretry}>Try again</Button>
</div>

<style>
  .retry {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    text-align: center;
    font-size: 12.5px;
    color: var(--muted);
  }
</style>
