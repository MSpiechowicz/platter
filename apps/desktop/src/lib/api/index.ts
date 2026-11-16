import { hooks } from '../hooks';
import type { Api } from './client';
import { createMockApi, MOCK_VOLUMES_WITH_SUBVOLUMES } from './mock';

/** `tauri.ts` is written by the desktop backend package; until it exists the glob is empty. */
const tauriModules = import.meta.glob<{ createTauriApi: () => Api }>('./tauri.ts');

/** An `Api` that loads its implementation on first use, so the Tauri module is a separate chunk. */
function lazyApi(load: () => Promise<Api>): Api {
  let loaded: Promise<Api> | undefined;
  const impl = (): Promise<Api> => (loaded ??= load());

  return {
    listVolumes: async () => (await impl()).listVolumes(),
    pickFolder: async () => (await impl()).pickFolder(),
    startScan: async (options, onEvent) => (await impl()).startScan(options, onEvent),
    cancelScan: async (scan) => (await impl()).cancelScan(scan),
    dropScan: async (scan) => (await impl()).dropScan(scan),
    getNode: async (scan, node) => (await impl()).getNode(scan, node),
    getChildren: async (scan, node, offset, limit, sort) =>
      (await impl()).getChildren(scan, node, offset, limit, sort),
    getLod: async (scan, node, maxNodes, maxDepth) => (await impl()).getLod(scan, node, maxNodes, maxDepth),
    getPath: async (scan, node) => (await impl()).getPath(scan, node),
    getSummary: async (scan, node) => (await impl()).getSummary(scan, node),
    getSuggestions: async (scan) => (await impl()).getSuggestions(scan),
    openInFileManager: async (scan, node) => (await impl()).openInFileManager(scan, node),
    openExternal: async (link) => (await impl()).openExternal(link),
  };
}

function selectApi(): Api {
  const insideTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
  const loadTauri = tauriModules['./tauri.ts'];

  if (insideTauri && loadTauri) {
    return lazyApi(async () => (await loadTauri()).createTauriApi());
  }

  if (insideTauri) {
    console.warn('Running inside Tauri but src/lib/api/tauri.ts is missing; using the mock API.');
  }

  return createMockApi({
    durationMs: hooks.static ? 0 : 2000,
    volumes: hooks.mounts ? MOCK_VOLUMES_WITH_SUBVOLUMES : undefined,
    holdAt: hooks.view === 'scanning' ? 0.62 : undefined,
    dominant: hooks.dominant,
  });
}

export const api: Api = selectApi();
