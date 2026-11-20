<script lang="ts">
  import { diskAccess } from '../../state/access.svelte';
  import Button from '../ui/Button.svelte';
  import Banner from './Banner.svelte';
</script>

<Banner
  icon="lock"
  title="Platter needs Full Disk Access"
  said="Platter needs Full Disk Access. Without it some folders are hidden and cannot be cleaned."
>
  Without it macOS hides some folders and blocks cleaning them. Switch Platter on under Privacy &amp;
  Security, Full Disk Access. After an update, switch it on again.

  {#snippet actions()}
    <Button onclick={() => diskAccess.later()}>Not now</Button>
    <Button variant="primary" onclick={() => void diskAccess.openSettings()}>Open System Settings</Button>
  {/snippet}
</Banner>
