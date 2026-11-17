import { hooks } from '../hooks';
import type { Api } from './client';
import { createMockApi, MOCK_VOLUMES_WITH_SUBVOLUMES } from './mock';
import type { MockAdvisorOptions } from './mock-advisor';
import { DEMO_USER_RULES } from './mock-user-rules';

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
    getChildren: async (scan, node, offset, limit, sort, query) =>
      (await impl()).getChildren(scan, node, offset, limit, sort, query),
    getLod: async (scan, node, maxNodes, maxDepth) => (await impl()).getLod(scan, node, maxNodes, maxDepth),
    getPath: async (scan, node) => (await impl()).getPath(scan, node),
    getSummary: async (scan, node) => (await impl()).getSummary(scan, node),
    getSuggestions: async (scan) => (await impl()).getSuggestions(scan),
    planCleanup: async (scan, suggestionIds) => (await impl()).planCleanup(scan, suggestionIds),
    planMarkedCleanup: async (scan, nodes) => (await impl()).planMarkedCleanup(scan, nodes),
    startCleanup: async (plan, onEvent) => (await impl()).startCleanup(plan, onEvent),
    cancelCleanup: async (cleanup) => (await impl()).cancelCleanup(cleanup),
    undoCleanup: async (cleanup) => (await impl()).undoCleanup(cleanup),
    advisorStatus: async () => (await impl()).advisorStatus(),
    watchAdvisor: async (onEvent) => (await impl()).watchAdvisor(onEvent),
    configureAdvisor: async (options) => (await impl()).configureAdvisor(options),
    clearQuestionLog: async () => (await impl()).clearQuestionLog(),
    answerGpuProblem: async (id, choice) => (await impl()).answerGpuProblem(id, choice),
    clearGpuLog: async () => (await impl()).clearGpuLog(),
    startModelDownload: async () => (await impl()).startModelDownload(),
    cancelModelDownload: async () => (await impl()).cancelModelDownload(),
    loadModel: async () => (await impl()).loadModel(),
    unloadModel: async () => (await impl()).unloadModel(),
    labelFolders: async (scan) => (await impl()).labelFolders(scan),
    cancelLabelling: async () => (await impl()).cancelLabelling(),
    getAdvisorLabels: async (scan) => (await impl()).getAdvisorLabels(scan),
    askAdvisor: async (scan, node, units) => (await impl()).askAdvisor(scan, node, units),
    askScan: async (scan, question, units, pick) => (await impl()).askScan(scan, question, units, pick),
    findFolders: async (scan, query, limit) => (await impl()).findFolders(scan, query, limit),
    listUserRules: async () => (await impl()).listUserRules(),
    addUserRule: async (scan, node, verdict) => (await impl()).addUserRule(scan, node, verdict),
    removeUserRule: async (id) => (await impl()).removeUserRule(id),
    resetUserRules: async () => (await impl()).resetUserRules(),
    openInFileManager: async (scan, node) => (await impl()).openInFileManager(scan, node),
    openExternal: async (link) => (await impl()).openExternal(link),
  };
}

/** The advisor mock's options the URL asks for: the graphics card, and an incident on it. */
function advisorHooks(): MockAdvisorOptions | undefined {
  const options: MockAdvisorOptions = {};

  if (hooks.gpu) {
    options.gpu = hooks.gpu;
  }

  if (hooks.gpuProblem) {
    options.gpuIncident = { what: hooks.gpuProblem, pending: true };
  } else if (hooks.gpuLast) {
    options.gpuIncident = { what: 'crash', pending: false };
  }

  return Object.keys(options).length > 0 ? options : undefined;
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
    warnings: hooks.warn ? ['mountTableUnavailable'] : undefined,
    advisor: advisorHooks(),
    userRules:
      hooks.rules === 'broken'
        ? { broken: true }
        : hooks.rules === 'seed'
          ? { rules: DEMO_USER_RULES }
          : undefined,
  });
}

export const api: Api = selectApi();
