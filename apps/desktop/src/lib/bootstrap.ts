import { hooks } from './hooks';
import { scan } from './state/scan.svelte';
import { settings } from './state/settings.svelte';
import { showDemoUpdate } from './state/update.svelte';
import { ui } from './state/ui.svelte';

/** Applies the test hooks from the URL (see hooks.ts) and loads what the first screen needs. */
export async function bootstrap(): Promise<void> {
  if (hooks.mini) {
    // Assigned directly, so the choice is not saved.
    settings.sidebarCollapsed = true;
  }

  if (hooks.update) {
    showDemoUpdate();
  }

  if (hooks.view === 'settings') {
    ui.go(hooks.view);
  }

  if (hooks.overlays) {
    showOverlays();
  }

  await scan.loadVolumes();

  if (
    hooks.view === 'overview' ||
    hooks.view === 'scanning' ||
    hooks.view === 'explorer' ||
    hooks.view === 'cleanup'
  ) {
    const root = hooks.root ?? scan.volumes[0]?.mountPoint ?? '/';

    // The Cleanup view's Scan step is on screen while the scan runs, so its page is chosen before it starts.
    if (hooks.view === 'scanning' && hooks.cleanup) {
      ui.go('cleanup');
    }

    await scan.start({ root, oneFileSystem: settings.oneFileSystem });

    // A finished scan opens its Overview, so the page is chosen after it.
    if (hooks.view === 'explorer' || hooks.view === 'cleanup') {
      await untilScanEnds();
      ui.go(hooks.view);
    }
  }

  if (hooks.view === 'start' && hooks.ready) {
    await scan.start({
      root: hooks.root ?? scan.volumes[0]?.mountPoint ?? '/',
      oneFileSystem: settings.oneFileSystem,
    });
    await untilScanEnds();
    ui.go('scan');
  }
}

async function untilScanEnds(): Promise<void> {
  while (scan.state.status === 'scanning') {
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

/** Opens what only appears on hover or after an action, so a screenshot can show it. */
function showOverlays(): void {
  ui.toast('Saved', 'The setting applies from now on.');
  ui.toast('Scan failed', 'Permission denied while reading /root.', 'error');

  setTimeout(() => {
    document.querySelector('[aria-label="What is new"]')?.dispatchEvent(new MouseEvent('mouseenter'));
  }, 500);
}
