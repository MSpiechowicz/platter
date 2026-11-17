<script lang="ts">
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Dropdown from '../../components/ui/Dropdown.svelte';
  import Field from '../../components/ui/Field.svelte';
  import { formatAgo } from '../../format';
  import { delayed } from '../../state/delayed.svelte';
  import { settings, type UpdateSchedule } from '../../state/settings.svelte';
  import { ui } from '../../state/ui.svelte';
  import { updater } from '../../state/update.svelte';

  const SCHEDULE_OPTIONS: { value: UpdateSchedule; label: string }[] = [
    { value: 'sixHours', label: 'Every 6 hours' },
    { value: 'daily', label: 'Daily' },
    { value: 'weekly', label: 'Weekly' },
    { value: 'never', label: 'Never' },
  ];

  // A check the user asked for shows its spinner at once, and the spinner stays at least a second. The result
  // replaces it only after that.
  const spinner = delayed(() => updater.checking, 0);
  const busy = $derived(updater.checking || spinner.value);

  /** The line under the version: what the last check found, or when one last ran. */
  const status = $derived.by(() => {
    if (busy) {
      return 'Checking for updates…';
    }

    const result = updater.result;

    if (result?.kind === 'upToDate') {
      return 'Platter is up to date.';
    }

    if (result?.kind === 'found') {
      return `Version ${result.version} is available.`;
    }

    if (result?.kind === 'error') {
      return result.message;
    }

    return updater.checkedAt === null
      ? 'Not checked yet.'
      : `Last checked ${formatAgo(Date.now() - updater.checkedAt)}.`;
  });
</script>

<Card title="Updates">
  <Field label={updater.current ? `Platter ${updater.current}` : 'Platter'} description={status}>
    {#snippet control()}
      <div class="check">
        {#if busy}
          <svg class="spin" viewBox="0 0 48 48" role="status" aria-label="Checking for updates">
            <circle cx="24" cy="24" r="13.5" fill="none" stroke="#ffffff1f" stroke-width="13" />
            <g
              ><path
                d="M32.77,6.02A20,20 0 0 1 40.96,13.40L29.94,20.29A7,7 0 0 0 27.07,17.71Z"
                fill="#3ec5d8"
              /></g
            >
          </svg>
        {/if}
        <Button disabled={busy} onclick={() => updater.check({ manual: true })}>Check now</Button>
      </div>
    {/snippet}
  </Field>

  <Field
    label="Check for updates"
    description="Platter checks in the background while it is open. Check now always works."
  >
    {#snippet control()}
      <Dropdown
        options={SCHEDULE_OPTIONS}
        value={settings.updateCheck}
        label="Check for updates"
        onchange={(value) => {
          settings.set('updateCheck', value);
          ui.toast('Saved', 'The setting applies from now on.');
        }}
      />
    {/snippet}
  </Field>

  {#if updater.canInstall === false}
    <p class="package">
      A package manager installs updates for this copy of Platter. Platter tells you when one is out.
    </p>
  {/if}
</Card>

<style>
  .check {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .spin {
    width: 22px;
    height: 22px;
    flex: none;
  }

  .spin g {
    transform-origin: 24px 24px;
    animation: spinrot 1.1s cubic-bezier(0.5, 0.15, 0.5, 0.85) infinite;
  }

  @keyframes spinrot {
    to {
      transform: rotate(360deg);
    }
  }

  .package {
    padding-top: 11px;
    border-top: 1px solid var(--line);
    font-size: 12px;
    line-height: 1.5;
    color: var(--muted);
  }
</style>
