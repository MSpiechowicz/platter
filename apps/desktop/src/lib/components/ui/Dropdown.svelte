<script lang="ts" generics="T extends string | number">
  import { tick } from 'svelte';
  import Icon from './Icon.svelte';

  interface Option {
    value: T;
    label: string;
  }

  interface Props {
    options: readonly Option[];
    value: T;
    /** Accessible name of the control. */
    label: string;
    onchange?: (value: T) => void;
  }

  let { options, value = $bindable(), label, onchange }: Props = $props();

  let open = $state(false);
  let root: HTMLDivElement | undefined = $state();
  let trigger: HTMLButtonElement | undefined = $state();
  let menu: HTMLDivElement | undefined = $state();

  const current = $derived(options.find((option) => option.value === value));

  function optionButtons(): HTMLButtonElement[] {
    return menu ? [...menu.querySelectorAll<HTMLButtonElement>('[role="option"]')] : [];
  }

  async function show(): Promise<void> {
    open = true;
    await tick();

    const buttons = optionButtons();
    (buttons.find((button) => button.getAttribute('aria-selected') === 'true') ?? buttons[0])?.focus();
  }

  function hide(restoreFocus: boolean): void {
    open = false;

    if (restoreFocus) {
      trigger?.focus();
    }
  }

  function select(option: Option): void {
    if (option.value !== value) {
      value = option.value;
      onchange?.(option.value);
    }

    hide(true);
  }

  function onMenuKey(event: KeyboardEvent): void {
    const buttons = optionButtons();
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);

    switch (event.key) {
      case 'ArrowDown':
        buttons[Math.min(buttons.length - 1, at + 1)]?.focus();
        break;
      case 'ArrowUp':
        buttons[Math.max(0, at - 1)]?.focus();
        break;
      case 'Home':
        buttons[0]?.focus();
        break;
      case 'End':
        buttons[buttons.length - 1]?.focus();
        break;
      case 'Escape':
        hide(true);
        break;
      case 'Tab':
        hide(false);
        return;
      default:
        return;
    }

    event.preventDefault();
    event.stopPropagation();
  }

  function onWindowClick(event: MouseEvent): void {
    if (open && root && !root.contains(event.target as Node)) {
      hide(false);
    }
  }
</script>

<svelte:window onclick={onWindowClick} />

<div class="dd" class:open bind:this={root}>
  <button
    type="button"
    class="trigger"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-label="{label}: {current?.label ?? ''}"
    bind:this={trigger}
    onclick={() => (open ? hide(false) : show())}
  >
    <span>{current?.label ?? ''}</span>
    <Icon name="chevron" size={12} stroke={1.8} />
  </button>

  <!-- Invisible copy of the menu: its width makes the button as wide as the longest option. -->
  <div class="panel sizer" aria-hidden="true">
    {#each options as option (option.value)}
      <div class="item"><i></i>{option.label}</div>
    {/each}
  </div>

  {#if open}
    <div
      class="panel menu"
      role="listbox"
      tabindex="-1"
      aria-label={label}
      bind:this={menu}
      onkeydown={onMenuKey}
    >
      {#each options as option (option.value)}
        {@const selected = option.value === value}
        <button
          type="button"
          role="option"
          class="item"
          class:on={selected}
          aria-selected={selected}
          onclick={() => select(option)}
        >
          <i><Icon name="check" size={12} stroke={1.8} /></i>{option.label}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .dd {
    position: relative;
    flex: none;
    display: inline-grid;
  }

  .dd > .trigger,
  .dd > .sizer {
    grid-area: 1 / 1;
  }

  .trigger {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    gap: 10px;
    background: var(--card2);
    border: 1px solid var(--line);
    border-radius: 7px;
    color: var(--text);
    font-size: 13px;
    padding: 6px 10px 6px 12px;
    white-space: nowrap;
    transition: border-color 0.15s;
  }

  .trigger:hover,
  .open > .trigger {
    border-color: var(--accent);
  }

  .trigger :global(svg) {
    color: var(--muted);
    transition: transform 0.2s;
  }

  .open > .trigger :global(svg) {
    transform: rotate(180deg);
  }

  .panel {
    background: var(--card2);
    border: 1px solid var(--line);
    border-radius: 9px;
    padding: 5px;
  }

  .sizer {
    visibility: hidden;
    height: 0;
    overflow: hidden;
    pointer-events: none;
    width: max-content;
  }

  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    width: 100%;
    z-index: 30;
    box-shadow: 0 14px 40px var(--shadow-2);
    animation: rise 0.16s var(--ease-out);
  }

  .item {
    display: grid;
    grid-template-columns: 14px 1fr;
    gap: 8px;
    align-items: center;
    width: 100%;
    text-align: left;
    padding: 7px 12px 7px 8px;
    border-radius: 6px;
    font-size: 13px;
    color: var(--muted);
    white-space: nowrap;
  }

  button.item:hover,
  :global(:root:not([data-input='pointer'])) button.item:focus-visible {
    background: var(--hover);
    color: var(--text);
    outline: 0;
  }

  .item.on {
    color: var(--text);
  }

  .item i {
    display: grid;
    place-items: center;
    color: var(--accent);
    visibility: hidden;
  }

  .item.on i {
    visibility: visible;
  }
</style>
