import { Channel, invoke } from '@tauri-apps/api/core';

import { ApiRequestError, isApiError, type Api } from './client';
import type {
  CleanupOptions,
  AdvisorAnswerDto,
  AdvisorLabelsDto,
  AdvisorOptionsDto,
  AdvisorStatusDto,
  ChildrenPage,
  CleanupEvent,
  CleanupId,
  CleanupPlanDto,
  ExternalLink,
  EntryMatchDto,
  FolderMatchDto,
  GpuProblemChoice,
  HistoryOverviewDto,
  HistoryRestoreDto,
  LodNode,
  NodeDto,
  NodeId,
  PathDto,
  PlanId,
  ScanAnswerDto,
  ScanEvent,
  ScanId,
  ScanOptions,
  SizeUnits,
  SortKey,
  SuggestionsDto,
  SummaryDto,
  UpdateCheck,
  UpdateProgress,
  UndoDto,
  UserRulesDto,
  UserVerdict,
  VolumeDto,
} from './types';

/**
 * Turns whatever a rejected `invoke` carries into an `ApiRequestError`.
 * Commands reject with a plain `{ code, message }` object; Tauri itself (bad arguments, missing
 * command) rejects with a string.
 */
function toApiError(reason: unknown): ApiRequestError {
  if (isApiError(reason)) {
    return new ApiRequestError(reason.code, reason.message);
  }

  if (reason instanceof Error) {
    return new ApiRequestError('io', reason.message);
  }

  return new ApiRequestError('io', typeof reason === 'string' ? reason : 'The backend request failed.');
}

/** `invoke` whose rejections are always `ApiRequestError`. Argument keys are the camelCase form of the Rust parameter names. */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (reason) {
    throw toApiError(reason);
  }
}

/** For commands that return nothing (Rust `()` arrives as `null`). */
async function callVoid(command: string, args?: Record<string, unknown>): Promise<void> {
  await call<unknown>(command, args);
}

export function createTauriApi(): Api {
  return {
    listVolumes: () => call<VolumeDto[]>('list_volumes'),

    pickFolder: () => call<string | null>('pick_folder'),

    pickExcludedFolder: () => call<string | null>('pick_excluded_folder'),

    startScan(options: ScanOptions, onEvent: (event: ScanEvent) => void): Promise<ScanId> {
      const channel = new Channel<ScanEvent>();
      channel.onmessage = onEvent;

      return call<ScanId>('start_scan', { options, onEvent: channel });
    },

    cancelScan: (scan: ScanId) => callVoid('cancel_scan', { scan }),

    dropScan: (scan: ScanId) => callVoid('drop_scan', { scan }),

    getNode: (scan: ScanId, node: NodeId) => call<NodeDto>('get_node', { scan, node }),

    getChildren: (scan: ScanId, node: NodeId, offset: number, limit: number, sort: SortKey, query?: string) =>
      call<ChildrenPage>('get_children', {
        scan,
        node,
        offset,
        limit,
        sort,
        ...(query ? { query } : {}),
      }),

    getLod: (scan: ScanId, node: NodeId, maxNodes: number, maxDepth: number) =>
      call<LodNode>('get_lod', { scan, node, maxNodes, maxDepth }),

    getPath: (scan: ScanId, node: NodeId) => call<PathDto>('get_path', { scan, node }),

    getSummary: (scan: ScanId, node: NodeId) => call<SummaryDto>('get_summary', { scan, node }),

    getSuggestions: (scan: ScanId, units?: SizeUnits) =>
      call<SuggestionsDto>('get_suggestions', { scan, units }),

    planCleanup: (scan: ScanId, suggestionIds: string[], options?: CleanupOptions) =>
      call<CleanupPlanDto>('plan_cleanup', { scan, suggestions: suggestionIds, options }),

    planMarkedCleanup: (scan: ScanId, nodes: NodeId[], options?: CleanupOptions) =>
      call<CleanupPlanDto>('plan_marked_cleanup', { scan, nodes, options }),

    startCleanup(plan: PlanId, onEvent: (event: CleanupEvent) => void): Promise<CleanupId> {
      const channel = new Channel<CleanupEvent>();
      channel.onmessage = onEvent;

      return call<CleanupId>('start_cleanup', { plan, onEvent: channel });
    },

    cancelCleanup: (cleanup: CleanupId) => callVoid('cancel_cleanup', { cleanup }),

    undoCleanup: (cleanup: CleanupId) => call<UndoDto[]>('undo_cleanup', { cleanup }),

    historyOverview: (days: number) => call<HistoryOverviewDto>('history_overview', { days }),

    restoreHistoryCleanup: (cleanup: number) =>
      call<HistoryRestoreDto>('restore_history_cleanup', { cleanup }),

    advisorStatus: () => call<AdvisorStatusDto>('advisor_status'),

    watchAdvisor(onEvent: (status: AdvisorStatusDto) => void): Promise<void> {
      const channel = new Channel<AdvisorStatusDto>();
      channel.onmessage = onEvent;

      return callVoid('watch_advisor', { onEvent: channel });
    },

    configureAdvisor: (options: AdvisorOptionsDto) =>
      call<AdvisorStatusDto>('configure_advisor', { options }),

    clearQuestionLog: () => callVoid('clear_question_log'),

    answerGpuProblem: (id: number, choice: GpuProblemChoice) =>
      call<AdvisorStatusDto>('answer_gpu_problem', { id, choice }),

    clearGpuLog: () => callVoid('clear_gpu_log', {}),

    startModelDownload: () => callVoid('start_model_download'),

    cancelModelDownload: () => callVoid('cancel_model_download'),

    loadModel: () => call<AdvisorStatusDto>('load_model'),

    unloadModel: () => call<AdvisorStatusDto>('unload_model'),

    labelFolders: (scan: ScanId) => callVoid('label_folders', { scan }),

    cancelLabelling: () => callVoid('cancel_labelling'),

    getAdvisorLabels: (scan: ScanId) => call<AdvisorLabelsDto>('get_advisor_labels', { scan }),

    askAdvisor: (scan: ScanId, node: NodeId, units?: SizeUnits) =>
      call<AdvisorAnswerDto>('ask_advisor', units === undefined ? { scan, node } : { scan, node, units }),

    askScan: (scan: ScanId, question: string, units: SizeUnits, pick?: number) =>
      call<ScanAnswerDto>(
        'ask_scan',
        pick === undefined ? { scan, question, units } : { scan, question, units, pick },
      ),

    findFolders: (scan: ScanId, query: string, limit: number) =>
      call<FolderMatchDto[]>('find_folders', { scan, query, limit }),

    findEntries: (scan: ScanId, query: string, limit: number) =>
      call<EntryMatchDto[]>('find_entries', { scan, query, limit }),

    listUserRules: () => call<UserRulesDto>('list_user_rules'),

    addUserRule: (scan: ScanId, node: NodeId, verdict: UserVerdict) =>
      call<UserRulesDto>('add_user_rule', { scan, node, verdict }),

    removeUserRule: (id: number) => call<UserRulesDto>('remove_user_rule', { id }),

    resetUserRules: () => call<UserRulesDto>('reset_user_rules'),

    openInFileManager: (scan: ScanId, node: NodeId) => callVoid('open_in_file_manager', { scan, node }),

    checkUpdate: () => call<UpdateCheck>('check_update'),

    installUpdate(onProgress: (progress: UpdateProgress) => void): Promise<void> {
      const channel = new Channel<UpdateProgress>();
      channel.onmessage = onProgress;

      return callVoid('install_update', { onProgress: channel });
    },

    restartApp: () => callVoid('restart_app'),

    openExternal: (link: ExternalLink) => callVoid('open_external', { link }),
  };
}
