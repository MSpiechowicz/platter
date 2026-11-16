import { Channel, invoke } from '@tauri-apps/api/core';

import { ApiRequestError, isApiError, type Api } from './client';
import type {
  ChildrenPage,
  ExternalLink,
  LodNode,
  NodeDto,
  NodeId,
  PathDto,
  ScanEvent,
  ScanId,
  ScanOptions,
  SortKey,
  SuggestionsDto,
  SummaryDto,
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

    startScan(options: ScanOptions, onEvent: (event: ScanEvent) => void): Promise<ScanId> {
      const channel = new Channel<ScanEvent>();
      channel.onmessage = onEvent;

      return call<ScanId>('start_scan', { options, onEvent: channel });
    },

    cancelScan: (scan: ScanId) => callVoid('cancel_scan', { scan }),

    dropScan: (scan: ScanId) => callVoid('drop_scan', { scan }),

    getNode: (scan: ScanId, node: NodeId) => call<NodeDto>('get_node', { scan, node }),

    getChildren: (scan: ScanId, node: NodeId, offset: number, limit: number, sort: SortKey) =>
      call<ChildrenPage>('get_children', { scan, node, offset, limit, sort }),

    getLod: (scan: ScanId, node: NodeId, maxNodes: number, maxDepth: number) =>
      call<LodNode>('get_lod', { scan, node, maxNodes, maxDepth }),

    getPath: (scan: ScanId, node: NodeId) => call<PathDto>('get_path', { scan, node }),

    getSummary: (scan: ScanId, node: NodeId) => call<SummaryDto>('get_summary', { scan, node }),

    getSuggestions: (scan: ScanId) => call<SuggestionsDto>('get_suggestions', { scan }),

    openInFileManager: (scan: ScanId, node: NodeId) => callVoid('open_in_file_manager', { scan, node }),

    openExternal: (link: ExternalLink) => callVoid('open_external', { link }),
  };
}
