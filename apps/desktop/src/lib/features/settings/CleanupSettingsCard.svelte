<script lang="ts">
  import type { KeepPackageVersions } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import Dropdown from '../../components/ui/Dropdown.svelte';
  import Field from '../../components/ui/Field.svelte';
  import Toggle from '../../components/ui/Toggle.svelte';
  import { platform } from '../../state/platform.svelte';
  import { settings } from '../../state/settings.svelte';
  import { ui } from '../../state/ui.svelte';

  const KEEP_OPTIONS: { value: KeepPackageVersions; label: string }[] = [
    { value: 1, label: '1' },
    { value: 2, label: '2' },
    { value: 3, label: '3' },
  ];

  function saved(): void {
    ui.toast('Saved', 'The next cleanup uses it.');
  }
</script>

<!-- On macOS cleaning only moves folders to the Trash: no pacman cache and no tool with administrator rights, so the card has nothing to set. -->
{#if !platform.isMac}
  <Card title="Cleanup">
    <!-- Pacman is a Linux tool: the row has nothing to set on Windows. -->
    {#if !platform.isWindows}
      <Field
        label="Keep package versions"
        description="How many old versions of each package stay in the pacman cache. Other package managers clear their whole cache."
      >
        {#snippet control()}
          <Dropdown
            options={KEEP_OPTIONS}
            value={settings.keepPackageVersions}
            label="Keep package versions"
            onchange={(value) => {
              settings.set('keepPackageVersions', value);
              saved();
            }}
          />
        {/snippet}
      </Field>
    {/if}

    <Field
      label="Ask for administrator rights when needed"
      description="Used for system caches and logs, through the standard system prompt. Off leaves those out of every cleanup."
    >
      {#snippet control(labelId, descriptionId)}
        <Toggle
          checked={settings.askForAdmin}
          aria-labelledby={labelId}
          aria-describedby={descriptionId}
          onchange={(event) => {
            settings.set('askForAdmin', event.currentTarget.checked);
            saved();
          }}
        />
      {/snippet}
    </Field>
  </Card>
{/if}
