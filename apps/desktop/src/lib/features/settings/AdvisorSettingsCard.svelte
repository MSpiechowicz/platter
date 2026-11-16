<script lang="ts">
  import Bar from '../../components/ui/Bar.svelte';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Dropdown from '../../components/ui/Dropdown.svelte';
  import Field from '../../components/ui/Field.svelte';
  import Notice from '../../components/ui/Notice.svelte';
  import Toggle from '../../components/ui/Toggle.svelte';
  import { advisor } from '../../state/advisor.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { settings } from '../../state/settings.svelte';
  import { ui } from '../../state/ui.svelte';
  import { visibleText } from '../overview/display';
  import { downloadLabel, downloadShare, receivedText, UNLOAD_OPTIONS } from '../advisor/model';

  const status = $derived(advisor.status);
  /** The model file is there: only then can it be switched on. */
  const downloaded = $derived(
    status?.state === 'onDisk' || status?.state === 'loading' || status?.state === 'loaded',
  );
  const problem = $derived(advisor.failure ?? status?.problem);

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
        return `${name} · Loading into RAM…`;
      case 'loaded':
        return `${name} · Loaded${status.ramBytes === undefined ? '' : ` (${fmt.size(status.ramBytes)} in RAM)`}`;
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
      {/if}
    {/snippet}
  </Field>

  <!-- The row is there in every state, so the card does not move when a download starts or ends. -->
  <div class="prog">
    {#if status?.state === 'downloading' && status.download}
      <Bar value={downloadShare(status.download)} label="Download progress" />
    {/if}
  </div>

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

  <Field
    label="Keep a local log of questions"
    description="Each question and how Platter read it is kept in a small file on this machine, never sent anywhere. It helps to check how questions are understood."
  >
    {#snippet control(labelId, descriptionId)}
      <div class="log">
        <Button onclick={clearLog}>Clear log</Button>
        <Toggle
          checked={settings.advisorLogQuestions}
          aria-labelledby={labelId}
          aria-describedby={descriptionId}
          onchange={(event) => {
            void advisor.setLogQuestions(event.currentTarget.checked);
            saved();
          }}
        />
      </div>
    {/snippet}
  </Field>
</Card>

<style>
  .prog {
    height: 4px;
    margin: 0 0 11px;
  }

  .problem {
    margin: 0 0 11px;
  }

  .log {
    display: flex;
    align-items: center;
    gap: 14px;
  }
</style>
