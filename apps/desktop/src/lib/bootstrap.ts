import { setMockAdvisorState } from './api/mock-advisor';
import type { ScanId } from './api/types';
import { hooks } from './hooks';
import { advisor } from './state/advisor.svelte';
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

  // The Advisor view opens at once only without a scan; with one it opens when the scan (and the labels) are there.
  if (hooks.view === 'settings' || (hooks.view === 'advisor' && !advisorScan())) {
    ui.go(hooks.view);
  }

  // The advisor first: its options, then its status channel.
  await startAdvisor();

  if (hooks.overlays) {
    showOverlays();
  }

  await scan.loadVolumes();

  if (
    hooks.view === 'overview' ||
    hooks.view === 'scanning' ||
    hooks.view === 'explorer' ||
    hooks.view === 'cleanup' ||
    (hooks.view === 'advisor' && advisorScan())
  ) {
    const root = hooks.root ?? scan.volumes[0]?.mountPoint ?? '/';

    // The Cleanup view's Scan step is on screen while the scan runs, so its page is chosen before it starts.
    if (hooks.view === 'scanning' && hooks.cleanup) {
      ui.go('cleanup');
    }

    await scan.start({ root, oneFileSystem: settings.oneFileSystem });

    // A finished scan opens its Overview, so the page is chosen after it.
    if (hooks.view === 'explorer' || hooks.view === 'cleanup' || hooks.view === 'advisor') {
      await untilScanEnds();

      if (hooks.view === 'advisor' && hooks.labels) {
        await labelFolders();
      }

      if (hooks.view === 'advisor' && hooks.ask && scan.state.status === 'ready') {
        // Not awaited: a screenshot can show the question while it is being answered.
        void advisor.askScan(scan.state.scanId, hooks.ask);
      }

      if (hooks.view === 'advisor' && hooks.chat && scan.state.status === 'ready') {
        await seedChat(scan.state.scanId, hooks.chat);
      }

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

/** Some questions the demo disk answers with rows, asked in turn to fill a conversation. */
const DEMO_QUESTIONS = [
  'biggest folders',
  'what can I clean?',
  'biggest caches',
  'what did the model label?',
  'why is ~/Projekty so big?',
];

/** The `chat` hook: `count` questions, each answered before the next. */
async function seedChat(scanId: ScanId, count: number): Promise<void> {
  for (let index = 0; index < count; index++) {
    await advisor.askScan(scanId, DEMO_QUESTIONS[index % DEMO_QUESTIONS.length] as string);
  }
}

/** The Advisor view hooks that need a finished scan: `labels`, `ask`, or a `root` to scan. */
function advisorScan(): boolean {
  return hooks.labels || hooks.ask !== null || hooks.chat !== null || hooks.root !== null;
}

/**
 * Sends the stored options to the backend, puts the mock model into the state the URL asked for, subscribes to the
 * status. A hook for a model that is on disk switches it on in the settings (not
 * saved), so a state like `loaded` stays what it says.
 */
async function startAdvisor(): Promise<void> {
  // A model that is on disk is on in the settings; one that is not there yet is off, as for a user.
  // A graphics card problem waiting at start is about a model that is running, so the mock starts loaded.
  const target = hooks.advisor ?? (hooks.gpuProblem ? 'loaded' : null);

  if (target === 'disk' || target === 'loaded') {
    settings.advisorEnabled = true;
  }

  await advisor.configure();

  if (target !== null) {
    setMockAdvisorState(target);
  }

  // No download prompt at start: the user downloads the model from the Advisor view or Settings when they want it.
  await advisor.watch();
}

/** The `labels` hook: the mock model labels the finished scan, and the page opens once it has. */
async function labelFolders(): Promise<void> {
  if (scan.state.status !== 'ready') {
    return;
  }

  await advisor.label(scan.state.scanId);

  while (advisor.status?.labelling !== undefined) {
    await new Promise((resolve) => setTimeout(resolve, 20));
  }

  await advisor.loadLabels(scan.state.scanId);
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
