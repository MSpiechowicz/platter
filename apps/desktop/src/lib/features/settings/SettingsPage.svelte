<script lang="ts">
  import { api } from '../../api';
  import { describeError, isApiError } from '../../api/client';
  import PageHeader from '../../components/layout/PageHeader.svelte';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Field from '../../components/ui/Field.svelte';
  import Segmented from '../../components/ui/Segmented.svelte';
  import Toggle from '../../components/ui/Toggle.svelte';
  import type { SizeUnits } from '../../format';
  import { settings } from '../../state/settings.svelte';
  import { applyThemeChoice, type ThemeChoice } from '../../state/theme.svelte';
  import { ui } from '../../state/ui.svelte';
  import AboutCard from './AboutCard.svelte';
  import AdvisorSettingsCard from './AdvisorSettingsCard.svelte';
  import CleanupSettingsCard from './CleanupSettingsCard.svelte';
  import ExcludedPaths from './ExcludedPaths.svelte';
  import UpdatesSettingsCard from './UpdatesSettingsCard.svelte';

  const UNIT_OPTIONS: { value: SizeUnits; label: string }[] = [
    { value: 'GB', label: 'GB' },
    { value: 'GiB', label: 'GiB' },
  ];

  const THEME_OPTIONS: { value: ThemeChoice; label: string }[] = [
    { value: 'dark', label: 'Dark' },
    { value: 'light', label: 'Light' },
    { value: 'system', label: 'System' },
  ];

  function saved(): void {
    ui.toast('Saved', 'The setting applies from now on.');
  }

  let forgetting = $state(false);

  /** Deletes the scan kept on disk. The backend asks first in its own dialog, and a no changes nothing. */
  async function forgetSavedScan(): Promise<void> {
    forgetting = true;

    try {
      await api.forgetLastScan();
      ui.toast(
        'Saved scan forgotten',
        'The scan on screen stays until Platter closes. The next scan is kept again.',
      );
    } catch (error) {
      if (!isApiError(error) || error.code !== 'cancelled') {
        ui.toast('Could not forget the saved scan', describeError(error), 'error');
      }
    } finally {
      forgetting = false;
    }
  }
</script>

<PageHeader title="SETTINGS" sub="saved automatically" />

<div class="sgrid">
  <div class="col">
    <Card title="Scanning">
      <Field
        label="Stay on this disk"
        description="Skip other disks, network shares and removable drives mounted inside it."
      >
        {#snippet control(labelId, descriptionId)}
          <Toggle
            checked={settings.oneFileSystem}
            aria-labelledby={labelId}
            aria-describedby={descriptionId}
            onchange={(event) => {
              settings.set('oneFileSystem', event.currentTarget.checked);
              saved();
            }}
          />
        {/snippet}
      </Field>
      <Field
        label="Rescan when Platter opens"
        description="Platter opens the last scan as it was. Turn this on to scan its folder again at every start."
      >
        {#snippet control(labelId, descriptionId)}
          <Toggle
            checked={settings.rescanOnOpen}
            aria-labelledby={labelId}
            aria-describedby={descriptionId}
            onchange={(event) => {
              settings.set('rescanOnOpen', event.currentTarget.checked);
              saved();
            }}
          />
        {/snippet}
      </Field>
      <Field
        label="Saved scan"
        description="Platter keeps the last scan on disk, with the names of its files and folders, to open it at the next start."
      >
        {#snippet control(labelId, descriptionId)}
          <Button
            size="sm"
            disabled={forgetting}
            aria-describedby={`${labelId} ${descriptionId}`}
            onclick={() => void forgetSavedScan()}>Forget</Button
          >
        {/snippet}
      </Field>
      <ExcludedPaths />
    </Card>

    <AdvisorSettingsCard />

    <Card title="Appearance" grow>
      <Field label="Theme" description="System follows the light or dark setting of your desktop.">
        {#snippet control()}
          <Segmented
            options={THEME_OPTIONS}
            value={settings.theme}
            label="Theme"
            onchange={(choice) => {
              settings.set('theme', choice);
              applyThemeChoice(choice);
            }}
          />
        {/snippet}
      </Field>
      <Field label="Size units" description="GB counts in thousands, GiB in 1024s.">
        {#snippet control()}
          <Segmented
            options={UNIT_OPTIONS}
            value={settings.units}
            label="Size units"
            onchange={(units) => {
              settings.set('units', units);
              saved();
            }}
          />
        {/snippet}
      </Field>
      <Field label="Reduce motion" description="Turns off the build-up and count-up animations.">
        {#snippet control(labelId, descriptionId)}
          <Toggle
            checked={settings.reduceMotion}
            aria-labelledby={labelId}
            aria-describedby={descriptionId}
            onchange={(event) => {
              settings.set('reduceMotion', event.currentTarget.checked);
              saved();
            }}
          />
        {/snippet}
      </Field>
    </Card>
  </div>

  <div class="col">
    <CleanupSettingsCard />

    <UpdatesSettingsCard />

    <AboutCard />
  </div>
</div>

<style>
  .sgrid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 16px;
    align-items: stretch;
  }

  .col {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
  }

  @media (max-width: 1000px) {
    .sgrid {
      grid-template-columns: 1fr;
    }
  }
</style>
