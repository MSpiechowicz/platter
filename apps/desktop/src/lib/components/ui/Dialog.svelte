<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    open: boolean;
    title: string;
    description?: string;
    /** Body between the description and the buttons. */
    children?: Snippet;
    /** The buttons. Mark the safe default with `data-autofocus` so it gets the focus. */
    actions: Snippet;
    onclose?: () => void;
  }

  let { open = $bindable(false), title, description, children, actions, onclose }: Props = $props();

  let dialog: HTMLDialogElement | undefined = $state();

  const titleId = $props.id();

  // The native <dialog> owns the modal behaviour: focus trap, Escape, inert background, focus restore.
  $effect(() => {
    if (!dialog) {
      return;
    }

    if (open && !dialog.open) {
      dialog.showModal();
      dialog.querySelector<HTMLElement>('[data-autofocus]')?.focus();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });

  function handleClose(): void {
    open = false;
    onclose?.();
  }

  // A click on the backdrop lands on the <dialog> element itself, not on its content.
  function handleClick(event: MouseEvent): void {
    if (event.target === dialog) {
      dialog?.close();
    }
  }
</script>

<dialog class="dlg" bind:this={dialog} aria-labelledby={titleId} onclose={handleClose} onclick={handleClick}>
  {#if open}
    <h2 id={titleId}>{title}</h2>
    {#if description}<p>{description}</p>{/if}
    {@render children?.()}
    <div class="dbtn">{@render actions()}</div>
  {/if}
</dialog>

<style>
  .dlg {
    background: var(--card);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: 12px;
    padding: 22px;
    width: min(480px, calc(100vw - 32px));
    margin: auto;
    box-shadow: 0 24px 80px #000c;
  }

  .dlg::backdrop {
    background: #05080dcc;
  }

  .dlg[open] {
    animation: rise 0.22s var(--ease-out);
  }

  h2 {
    font-size: 17px;
    margin-bottom: 6px;
  }

  p {
    color: var(--muted);
    font-size: 13px;
    margin-bottom: 14px;
  }

  .dbtn {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    margin-top: 20px;
  }
</style>
