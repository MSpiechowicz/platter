<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import AppShell from './lib/components/layout/AppShell.svelte';
  import PageBoundary from './lib/components/layout/PageBoundary.svelte';
  import Sidebar, { type NavItem } from './lib/components/layout/Sidebar.svelte';
  import ToastHost from './lib/components/ui/ToastHost.svelte';
  import TooltipHost from './lib/components/ui/TooltipHost.svelte';
  import { bootstrap } from './lib/bootstrap';
  import AdvisorPage from './lib/features/advisor/AdvisorPage.svelte';
  import GpuProblemDialog from './lib/features/advisor/GpuProblemDialog.svelte';
  import CleanupPage from './lib/features/cleanup/CleanupPage.svelte';
  import Explorer from './lib/features/explorer/Explorer.svelte';
  import History from './lib/features/history/History.svelte';
  import OpeningScreen from './lib/features/opening/OpeningScreen.svelte';
  import OverviewPage from './lib/features/overview/OverviewPage.svelte';
  import ScanProgress from './lib/features/scan/ScanProgress.svelte';
  import SettingsPage from './lib/features/settings/SettingsPage.svelte';
  import StartPage from './lib/features/start/StartPage.svelte';
  import { advisor } from './lib/state/advisor.svelte';
  import { syncMotionAttribute } from './lib/state/motion.svelte';
  import {
    highlightedPage,
    isPageAvailable,
    pageAfterScan,
    resolveScreen,
    SCAN_FIRST_HINT,
  } from './lib/state/navigation';
  import { opening } from './lib/state/opening.svelte';
  import { scan } from './lib/state/scan.svelte';
  import { settings } from './lib/state/settings.svelte';
  import { ui, type Page } from './lib/state/ui.svelte';
  import { updater, type UpdateAction } from './lib/state/update.svelte';
  import { viewport } from './lib/state/viewport.svelte';

  const NAV: (NavItem & { id: Page })[] = [
    { id: 'scan', label: 'Scan', icon: 'scan' },
    { id: 'overview', label: 'Overview', icon: 'overview' },
    { id: 'explorer', label: 'Explorer', icon: 'explorer' },
    { id: 'cleanup', label: 'Cleanup', icon: 'cleanup' },
    { id: 'advisor', label: 'Advisor', icon: 'advisor' },
    { id: 'history', label: 'History', icon: 'history' },
    { id: 'settings', label: 'Settings', icon: 'settings' },
  ];

  syncMotionAttribute();

  onMount(() => {
    // Whatever goes wrong while starting, the app opens: the opening screen never stays up over an app it hides.
    bootstrap().catch((error: unknown) => {
      opening.skip();
      console.error('Start-up failed', error);
    });
  });

  // The menu's logo, which the opening screen's logo lands on.
  let brandLogo: HTMLElement | null = $state(null);

  // A narrow window always shows the icon rail.
  const collapsed = $derived(settings.sidebarCollapsed || viewport.narrow);
  const scanState = $derived(scan.state);
  const screen = $derived(resolveScreen(scanState, ui.page));

  const items = $derived(
    NAV.map((item) => {
      // The item whose page is on screen is never dimmed: Cleanup shows its own first step while a scan runs.
      const available = isPageAvailable(item.id, scanState.status) || item.id === highlightedPage(screen);

      return { ...item, disabled: !available, hint: available ? undefined : SCAN_FIRST_HINT };
    }),
  );

  // A scan that finishes takes the user to its Overview.
  let lastStatus = scan.state.status;

  $effect(() => {
    const status = scanState.status;

    if (status === 'ready' && lastStatus !== 'ready') {
      ui.go(pageAfterScan(untrack(() => ui.page)));
    }

    lastStatus = status;
  });

  // The Explorer's place belongs to one scan: a new scan, a failed or a cancelled one starts at the scan root.
  $effect(() => {
    if (scanState.status !== 'ready') {
      ui.forgetExplorer();
    }
  });

  function onUpdate(action: UpdateAction): void {
    if (action === 'install') {
      // The progress bar needs the full sidebar.
      settings.set('sidebarCollapsed', false);
      void updater.install();
    } else if (action === 'open') {
      void updater.openReleases();
    } else if (action === 'later') {
      updater.later();
      ui.toast('Update postponed', 'The notice comes back after the next update check.', 'info');
    } else {
      void updater.restart();
    }
  }
</script>

<AppShell {collapsed} phase={opening.phase}>
  {#snippet sidebar()}
    <Sidebar
      {items}
      active={highlightedPage(screen)}
      onselect={(id) => ui.go(id as Page)}
      {collapsed}
      arriving={opening.phase !== 'done'}
      bind:logo={brandLogo}
      ontoggle={viewport.narrow
        ? undefined
        : () => settings.set('sidebarCollapsed', !settings.sidebarCollapsed)}
      volume={scan.volumeInfo?.volume}
      advisorBytes={advisor.loaded ? advisor.status?.ramBytes : undefined}
      update={updater.info}
      updatePhase={updater.phase}
      updateProgress={updater.progress}
      updateIndeterminate={updater.indeterminate}
      onupdate={onUpdate}
    />
  {/snippet}

  <!-- One boundary per page: a page that fails shows a card, and going elsewhere starts afresh. -->
  {#if screen.id === 'settings'}
    <PageBoundary><SettingsPage /></PageBoundary>
  {:else if screen.id === 'history'}
    <PageBoundary><History /></PageBoundary>
  {:else if screen.id === 'advisor'}
    <PageBoundary><AdvisorPage ready={screen.scan} scanning={scanState.status === 'scanning'} /></PageBoundary
    >
  {:else if screen.id === 'progress'}
    <PageBoundary><ScanProgress running={screen.scan} /></PageBoundary>
  {:else if screen.id === 'explorer'}
    <PageBoundary><Explorer /></PageBoundary>
  {:else if screen.id === 'cleanup'}
    <PageBoundary><CleanupPage scan={screen.scan} /></PageBoundary>
  {:else if screen.id === 'overview'}
    <PageBoundary><OverviewPage ready={screen.scan} /></PageBoundary>
  {:else}
    <PageBoundary><StartPage /></PageBoundary>
  {/if}
</AppShell>

{#if opening.phase !== 'done'}
  <OpeningScreen target={brandLogo} />
{/if}

<GpuProblemDialog />
<ToastHost />
<TooltipHost />
