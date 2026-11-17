<script lang="ts">
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Dropdown from '../../components/ui/Dropdown.svelte';
  import Field from '../../components/ui/Field.svelte';
  import Notice from '../../components/ui/Notice.svelte';
  import Toggle from '../../components/ui/Toggle.svelte';
  import type { AdvisorStatusDto } from '../../api/types';
  import { advisor } from '../../state/advisor.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { settings } from '../../state/settings.svelte';
  import { ui } from '../../state/ui.svelte';
  import { visibleText } from '../overview/display';
  import {
    downloadLabel,
    gpuProblemSummary,
    receivedText,
    SWITCHED_OFF_HINT,
    UNLOAD_OPTIONS,
  } from '../advisor/model';

  const status = $derived(advisor.status);
  /** The model file is there: only then can it be switched on. */
  const downloaded = $derived(
    status?.state === 'onDisk' || status?.state === 'loading' || status?.state === 'loaded',
  );
  const problem = $derived(advisor.failure ?? status?.problem);
  const lastGpuProblem = $derived(advisor.lastGpuProblem);

  /** " (2.1 GB in RAM)", or the video memory instead while the model is on a graphics card. */
  function memory(loaded: AdvisorStatusDto): string {
    if (loaded.device?.kind === 'gpu' && loaded.videoBytes !== undefined) {
      return ` (${fmt.size(loaded.videoBytes)} video memory)`;
    }

    return loaded.ramBytes === undefined ? '' : ` (${fmt.size(loaded.ramBytes)} in RAM)`;
  }

  /** The state row's one-line description. */
  const description = $derived.by(() => {
    if (!status) {
      return 'Checking the model…';
    }

    const name = visibleText(status.model.name);

    switch (status.state) {
      case 'notDownloaded':
        return status.download
          ? `${name} · ${receivedText(status.download, fmt.size)} downloaded`
          : `${name} · Not downloaded`;
      case 'downloading':
        return `${name} · Downloading ${status.download ? receivedText(status.download, fmt.size) : '…'}`;
      case 'onDisk':
        return `${name} · On disk (${fmt.size(status.model.bytes)})`;
      case 'loading':
        return `${name} · Loading…`;
      case 'loaded':
        return `${name} · Loaded${memory(status)}`;
      default:
        return 'Not available in this build.';
    }
  });

  function saved(): void {
    ui.toast('Saved', 'The setting applies from now on.');
  }

  async function clearLog(): Promise<void> {
    if (await advisor.clearQuestionLog()) {
      ui.toast('Cleared', 'The question log was cleared.');
    }
  }

  async function clearGpuLog(): Promise<void> {
    if (await advisor.clearGpuLog()) {
      ui.toast('Cleared', 'The graphics card problem log was cleared.');
    }
  }
</script>

<Card title="Advisor">
  <Field
    label="Use the local model"
    description={downloaded ? 'Off: only the built-in rules label items.' : 'Download the model first.'}
  >
    {#snippet control(labelId, descriptionId)}
      <Toggle
        checked={downloaded && settings.advisorEnabled}
        disabled={!downloaded}
        aria-labelledby={labelId}
        aria-describedby={descriptionId}
        onchange={(event) => {
          void advisor.setEnabled(event.currentTarget.checked);
          saved();
        }}
      />
    {/snippet}
  </Field>

  <Field label="Model" {description}>
    {#snippet control()}
      {#if status?.state === 'notDownloaded'}
        <Button disabled={advisor.asking} onclick={() => advisor.download()}>
          {downloadLabel(status, fmt.size)}
        </Button>
      {:else if status?.state === 'downloading'}
        <Button onclick={() => advisor.cancelDownload()}>Cancel</Button>
      {:else if status?.state === 'onDisk' || status?.state === 'loading'}
        <!-- The same action as the Advisor page's header. -->
        <Button
          disabled={status.state === 'loading' || !advisor.enabled || advisor.switching}
          title={advisor.enabled ? undefined : SWITCHED_OFF_HINT}
          onclick={() => advisor.load()}
        >
          Load model
        </Button>
      {:else if status?.state === 'loaded'}
        <Button
          disabled={advisor.switching || status.labelling !== undefined}
          onclick={() => advisor.unload()}
        >
          Unload model
        </Button>
      {/if}
    {/snippet}
  </Field>

  {#if problem}
    <div class="problem"><Notice kind="error">{visibleText(problem)}</Notice></div>
  {/if}

  <Field label="Unload from RAM after">
    {#snippet control()}
      <Dropdown
        options={UNLOAD_OPTIONS}
        value={settings.advisorUnloadAfter}
        label="Unload from RAM after"
        onchange={(value) => {
          void advisor.setUnloadAfter(value);
          saved();
        }}
      />
    {/snippet}
  </Field>

  {#if status?.gpuSupported}
    <Field
      label="Use the graphics card"
      description="Faster when a graphics card with enough memory is there. Otherwise Platter uses the processor."
    >
      {#snippet control(labelId, descriptionId)}
        <Toggle
          checked={settings.advisorUseGpu}
          aria-labelledby={labelId}
          aria-describedby={descriptionId}
          onchange={(event) => {
            void advisor.setUseGpu(event.currentTarget.checked);
            saved();
          }}
        />
      {/snippet}
    </Field>
  {/if}

  {#if lastGpuProblem}
    <Field label="Last graphics card problem" description={gpuProblemSummary(lastGpuProblem)}>
      {#snippet control()}
        <div class="log">
          <Button aria-label="Clear the graphics card problem log" onclick={clearGpuLog}>Clear</Button>
        </div>
      {/snippet}
    </Field>
  {/if}

  <Field
    label="Keep a local log of questions"
    description="Each question and how Platter read it is kept in a small file on this machine, never sent anywhere. It helps to check how questions are understood."
  >
    {#snippet control(labelId, descriptionId)}
      <Toggle
        checked={settings.advisorLogQuestions}
        aria-labelledby={labelId}
        aria-describedby={descriptionId}
        onchange={(event) => {
          void advisor.setLogQuestions(event.currentTarget.checked);
          saved();
        }}
      />
    {/snippet}
  </Field>

  <Field label="Question log" description="Removes every question kept so far from this machine.">
    {#snippet control()}
      <Button onclick={clearLog}>Clear log</Button>
    {/snippet}
  </Field>
</Card>

<style>
  .problem {
    margin: 0 0 11px;
  }
</style>
