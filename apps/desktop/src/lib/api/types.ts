// Mirror of the DTO types in docs/architecture.md. Change the document first, then this file.

export type ScanId = number;
export type NodeId = number;

/** One of the fixed project pages the backend can open in the browser. The UI never passes a URL. */
export type ExternalLink = 'source' | 'issues' | 'license';

export interface VolumeDto {
  id: string;
  mountPoint: string;
  label: string;
  device: string;
  fsType: string;
  total: number;
  available: number;
  removable: boolean;
}

export interface ScanOptions {
  root: string;
  oneFileSystem: boolean;
}

/** Things the scan could not do properly; left out of the event when there are none. */
export type ScanWarning = 'mountTableUnavailable';

export type ScanEvent =
  | { type: 'progress'; files: number; dirs: number; bytes: number; errors: number; currentPath: string }
  | {
      type: 'finished';
      scanId: ScanId;
      rootId: NodeId;
      totals: Totals;
      elapsedMs: number;
      warnings?: ScanWarning[];
    }
  | { type: 'cancelled' }
  | { type: 'failed'; message: string };

export interface Totals {
  allocated: number;
  apparent: number;
  files: number;
  dirs: number;
  errors: number;
}

export type NodeKind = 'dir' | 'file' | 'symlink' | 'mount' | 'other' | 'collapsed';
export type Category =
  | 'video'
  | 'image'
  | 'audio'
  | 'document'
  | 'archive'
  | 'code'
  | 'build'
  | 'package'
  | 'app'
  | 'diskImage'
  | 'data'
  | 'other';

export interface NodeDto {
  id: NodeId;
  name: string;
  kind: NodeKind;
  allocated: number;
  apparent: number;
  files: number;
  hasChildren: boolean;
  category: Category;
  mtime: number;
  /** True for a folder the scan could not read; its size is unknown, not zero. The backend always sends it. */
  unreadable?: boolean;
  /** Why a mount was not entered. Only present for kind `mount`. */
  skipped?: SkipReason;
}

/** otherDisk: another filesystem; system: a virtual, network or FUSE filesystem; duplicate: same disk, already counted; unknown: could not be classified. */
export type SkipReason = 'otherDisk' | 'system' | 'duplicate' | 'unknown';

export type SortKey = 'sizeDesc' | 'sizeAsc' | 'nameAsc' | 'filesDesc' | 'mtimeDesc';

export interface ChildrenPage {
  parentId: NodeId;
  total: number;
  items: NodeDto[];
}

export interface LodNode {
  node: NodeDto;
  children: LodNode[];
}

export interface PathDto {
  path: string;
  segments: { id: NodeId; name: string }[];
}

export type AgeBucket = 'week' | 'month' | 'quarter' | 'year' | 'twoYears' | 'older';

export interface SummaryDto {
  node: NodeDto;
  byCategory: { category: Category; allocated: number; files: number }[];
  byAge: { bucket: AgeBucket; allocated: number; files: number }[];
  largestDirs: NodeDto[];
}

export type Verdict = 'safe' | 'review' | 'keep';

export interface SuggestionDto {
  id: string; // rule id, stable, e.g. 'pacman-cache'
  title: string; // 'Pacman package cache'
  path: string; // display path or pattern, e.g. '/var/cache/pacman/pkg' or '~/Projekty/**/target'
  verdict: Verdict;
  reason: string; // one or two plain sentences
  allocated: number; // bytes this suggestion covers
  items: number; // how many folders or files matched
  nodes: NodeId[]; // up to 20 largest matched nodes, largest first
  sharedFiles?: number; // files inside that have hard links; removing them may free more or less than shown
}

export interface SuggestionsDto {
  suggestions: SuggestionDto[]; // safe first, then review, then keep; largest first within each
  totals: { safe: number; review: number; keep: number };
}

// Errors are returned as `ApiError { code, message }` (prose in docs/architecture.md, "IPC contract").
export type ApiErrorCode = 'notFound' | 'cancelled' | 'io' | 'invalid';

export interface ApiError {
  code: ApiErrorCode;
  message: string;
}
