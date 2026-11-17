<script lang="ts">
  import PageHeader from '../../components/layout/PageHeader.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Field from '../../components/ui/Field.svelte';
  import Segmented from '../../components/ui/Segmented.svelte';
  import Toggle from '../../components/ui/Toggle.svelte';
  import type { SizeUnits } from '../../format';
  import { settings } from '../../state/settings.svelte';
  import { ui } from '../../state/ui.svelte';
  import AboutCard from './AboutCard.svelte';
  import AdvisorSettingsCard from './AdvisorSettingsCard.svelte';
  import UpdatesSettingsCard from './UpdatesSettingsCard.svelte';

  const UNIT_OPTIONS: { value: SizeUnits; label: string }[] = [
    { value: 'GB', label: 'GB' },
    { value: 'GiB', label: 'GiB' },
  ];

  function saved(): void {
    ui.toast('Saved', 'The setting applies from now on.');
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
    </Card>

    <AdvisorSettingsCard />

    <UpdatesSettingsCard />

    <Card title="Appearance" grow>
      <div>
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
      </div>
    </Card>
  </div>

  <AboutCard />
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
