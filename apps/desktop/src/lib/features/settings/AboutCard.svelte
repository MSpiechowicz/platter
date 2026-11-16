<script lang="ts">
  import { api } from '../../api';
  import Card from '../../components/ui/Card.svelte';
  import Field from '../../components/ui/Field.svelte';

  /** Software that ships inside the interface. The Rust side is listed in the source repository. */
  const LICENSES = [
    { name: 'Svelte', license: 'MIT' },
    { name: 'Tauri API (@tauri-apps/api)', license: 'MIT or Apache-2.0' },
  ];
</script>

<Card title="About" grow>
  <Field label="Platter" description="Version 0.1.0">
    {#snippet control()}
      <span class="muted">GPL-3.0-or-later</span>
    {/snippet}
  </Field>

  <p class="about">
    <b>Platter</b> is free software under the GNU General Public License, version 3 or later. Everything runs on
    this machine. The only network call is the optional model download.
  </p>
  <p class="links">
    <!-- Buttons, not links: target="_blank" does nothing inside the Tauri window, so the backend opens the page. -->
    <button type="button" class="link" onclick={() => api.openExternal('source')}>Source code</button> ·
    <button type="button" class="link" onclick={() => api.openExternal('issues')}>Report a problem</button> ·
    <button type="button" class="link" onclick={() => api.openExternal('license')}>License text</button>
  </p>

  <h4>Included software · Rust crates are listed in the repository</h4>
  {#each LICENSES as item (item.name)}
    <div class="row"><span>{item.name}</span><span class="muted">{item.license}</span></div>
  {/each}
</Card>

<style>
  .about,
  .links {
    font-size: 13px;
    color: var(--muted);
    line-height: 1.6;
  }

  .about {
    padding-top: 11px;
    border-top: 1px solid var(--line);
  }

  .about b {
    color: var(--text);
  }

  .links {
    margin-top: 8px;
    margin-bottom: 14px;
  }

  .link {
    color: var(--accent);
    font: inherit;
    padding: 0;
    border-radius: 2px;
  }

  .link:hover {
    color: var(--accent-hover);
    text-decoration: underline;
  }

  h4 {
    font-size: 11px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--muted);
    font-weight: 600;
    padding: 12px 0 4px;
    border-top: 1px solid var(--line);
  }

  .row {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 7px 0;
    font-size: 13px;
  }
</style>
