// Mirror of the DTO types in docs/architecture.md. Change the document first, then this file.

import type { SizeUnits } from '../format';

export type { SizeUnits };

export type ScanId = number;
export type NodeId = number;

/** One of the fixed project pages the backend can open in the browser. The UI never passes a URL. */
export type ExternalLink = 'source' | 'issues' | 'license' | 'releases';

/**
 * Whether macOS lets Platter into every folder (Full Disk Access). Always `granted` on Linux and Windows. `unknown`
 * when the check could not tell, and then no notice is shown.
 */
export type DiskAccess = 'granted' | 'missing' | 'unknown';

/** An update the release server offers. */
export interface UpdateOffer {
  version: string;
  /** What is new, one short line each (at most 12 lines of 200 characters). */
  notes: string[];
  /** Size of the download, when the release says. */
  sizeBytes: number | null;
  /** This copy can install it itself (an AppImage on Linux, the setup's copy on Windows). Otherwise the release page is offered. */
  canInstall: boolean;
}

/** The answer of an update check: the running version, and the offer when a newer one exists. */
export interface UpdateCheck {
  current: string;
  update: UpdateOffer | null;
}

/** Progress of `installUpdate`: the download (`total` is null when the size is unknown), then the install. */
export type UpdateProgress =
  { kind: 'downloading'; received: number; total: number | null } | { kind: 'installing' };

export interface VolumeDto {
  id: string;
  mountPoint: string;
  label: string;
  device: string;
  fsType: string;
  total: number;
  available: number;
  removable: boolean;
  /**
   * The filesystem compresses what is written to it (btrfs with `compress=`). Its files take less space on the disk
   * than the scan counts for them, so a cleanup frees less than their size. Absent means no.
   */
  compressed?: boolean;
}

export interface ScanOptions {
  root: string;
  oneFileSystem: boolean;
  /** Absolute paths of folders the scan leaves out (at most 64, each at most 4096 bytes). A missing one is ignored. */
  excluded?: string[];
}

/** The most folders a scan can leave out, and the longest path of one, in bytes. */
export const MAX_EXCLUDED = 64;
export const MAX_EXCLUDED_PATH_BYTES = 4096;

/** Things the scan could not do properly; left out of the event when there are none. */
export type ScanWarning = 'mountTableUnavailable' | 'excludedUnresolved';

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

/** The scan Platter kept from its last run, opened again. `outdated` is set when a cleanup ran on it since. */
export interface LastScanDto {
  scanId: ScanId;
  rootId: NodeId;
  totals: Totals;
  elapsedMs: number;
  /** Left out when there are none, as in the `finished` event. */
  warnings?: ScanWarning[];
  options: ScanOptions;
  outdated: boolean;
  /** When the scan finished, in unix seconds. */
  scannedAt: number;
}

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

/**
 * otherDisk: another filesystem; system: a virtual, network or FUSE filesystem; duplicate: same disk, already counted;
 * excluded: a folder the user leaves out of scans (Settings); unknown: could not be classified.
 */
export type SkipReason = 'otherDisk' | 'system' | 'duplicate' | 'excluded' | 'unknown';

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

/** Who decided a verdict: the built-in `rule` table, the user's own rule, or the local `model`. */
export type SuggestionSource = 'rule' | 'user' | 'model';

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
  /** Who decided; missing means `rule` (the backend always sends it). */
  source?: SuggestionSource;
}

export interface SuggestionsDto {
  suggestions: SuggestionDto[]; // safe first, then review, then keep; largest first within each
  totals: { safe: number; review: number; keep: number };
}

export type PlanId = number;
export type CleanupId = number;

/** What cleaning one group of matches means. `trash` can be undone; the others cannot. */
export type CleanupActionKind = 'delete' | 'emptyFolder' | 'emptyTrash' | 'trash' | 'runTool';

/** The matches of one rule that a plan acts on. */
export interface PlanGroupDto {
  ruleId: string;
  title: string;
  verdict: Verdict;
  action: CleanupActionKind;
  items: number;
  allocated: number;
  /** The command a tool runs, as text; only for `runTool`. */
  command?: string;
  needsElevation: boolean;
  /** The first few paths (up to 20), for showing. */
  samplePaths: string[];
}

/** A match the plan will not touch, with the reason in plain words. */
export interface RefusedDto {
  ruleId: string;
  path: string;
  reason: string;
}

/** How many old versions of each pacman package stay in its cache. */
export type KeepPackageVersions = 1 | 2 | 3;

/** Options fixed into a cleanup plan when it is made (Settings, Cleanup card). */
export interface CleanupOptions {
  keepPackageVersions: KeepPackageVersions;
  /** Off: everything that needs the administrator password is left out of the plan. */
  askForAdmin: boolean;
}

export interface CleanupPlanDto {
  planId: PlanId;
  scanId: ScanId;
  /** The options the plan was made with. */
  options: CleanupOptions;
  groups: PlanGroupDto[];
  /**
   * At most 200 refusals; `refusedMore` counts the rest. Items left out because administrator rights are turned off
   * come first, with the reason "Needs administrator rights, which are turned off in Settings."
   */
  refused: RefusedDto[];
  refusedMore: number;
  /** Rule ids that matched nothing in this scan. */
  notFound: string[];
  allocated: number;
  /**
   * Starting the plan shows the backend's native confirmation: true whenever the plan has something to do (moving
   * to the trash is confirmed too). An empty plan cannot be started.
   */
  needsConfirmation: boolean;
}

export type OutcomeKind = 'removed' | 'trashed' | 'byTool' | 'skipped' | 'failed';

export interface OutcomeDto {
  kind: OutcomeKind;
  /** Why it was skipped or failed, or the command a tool ran. */
  detail?: string;
  /** Problems inside the target (entries that could not be removed, mounts left alone). */
  problems: number;
}

export type CleanupEvent =
  | { type: 'groupStarted'; ruleId: string; targets: number }
  | { type: 'targetDone'; ruleId: string; path: string; allocated: number; outcome: OutcomeDto }
  | { type: 'groupFinished'; ruleId: string }
  | {
      type: 'finished';
      cleanupId: CleanupId;
      /** The scan the cleanup ran on no longer describes the disk. */
      outdatedScan: ScanId;
      cleaned: number;
      withProblems: number;
      /** Free space after minus before; absent when it could not be measured. */
      freed?: number;
      cancelled: boolean;
      toolsNotRun: string[];
      /** Something was moved to the trash and `undoCleanup` can put it back. */
      canUndo: boolean;
    }
  | { type: 'failed'; message: string; outdatedScan: ScanId; canUndo: boolean; cleanupId: CleanupId };

export interface UndoDto {
  path: string;
  restored: boolean;
  error?: string;
}

/** What became of the items a cleanup moved to the trash. `notChecked`: too many items to look at, so Restore is still offered. */
export type HistoryStatus = 'inTrash' | 'restored' | 'noLongerInTrash' | 'permanent' | 'notChecked';

/** `command` is set for `tool` only, as it was run: "paccache -rk2". */
export interface HistoryMethodDto {
  kind: 'trash' | 'deleted' | 'emptiedTrash' | 'tool';
  command?: string;
}

export interface HistoryCleanupDto {
  id: number;
  /** Unix seconds. */
  at: number;
  /** The local day, `YYYY-MM-DD`. */
  day: string;
  /** What was cleaned: suggestion or rule titles, or folder names. */
  what: string[];
  freed: number;
  methods: HistoryMethodDto[];
  status: HistoryStatus;
  restorable: number;
}

export interface HistorySampleDto {
  /** The local day, `YYYY-MM-DD`. */
  day: string;
  used: number;
  total: number;
}

export interface HistoryOverviewDto {
  volume?: { id: string; label: string };
  days: number;
  today: string;
  /** Oldest first. `without` has one value for each sample: the used space had nothing been cleaned. */
  samples: HistorySampleDto[];
  without: number[];
  /** In the window, newest first. */
  cleanups: HistoryCleanupDto[];
  freed: number;
  cleanupCount: number;
  restorable: number;
  /** Left out when there are too few days, or the disk is not growing. */
  growthPerDay?: number;
  daysUntilFull?: number;
  /** Fewer than 7 samples, or samples that span fewer than 7 days. */
  thin: boolean;
  /** Why nothing is recorded (the history file cannot be used). */
  problem?: string;
}

export interface HistoryRestoreDto {
  restored: number;
  items: UndoDto[];
}

/** The operating system the app runs on (a closed set). The mock says `linux`, or `windows` with the `os` hook. */
export type OsName = 'linux' | 'windows' | 'macos';

/** `unavailable`: the runtime is not built into this app (macOS for now). */
export type ModelState = 'unavailable' | 'notDownloaded' | 'downloading' | 'onDisk' | 'loading' | 'loaded';

export type UnloadAfter = 'oneMinute' | 'fiveMinutes' | 'windowClose';

export interface AdvisorOptionsDto {
  enabled: boolean;
  unloadAfter: UnloadAfter;
  /** Keep a local log of the questions and how they were read. The backend keeps this in memory only. */
  logQuestions: boolean;
  /** Run the model on a graphics card when one with enough memory is there (default on); otherwise, or when it fails, on the processor. */
  useGpu: boolean;
}

/** Where the loaded model runs. `name` is the driver's description of the card, for `gpu` only. */
export interface DeviceDto {
  kind: 'gpu' | 'cpu';
  name?: string;
}

/** The one local model: Qwen3.5-4B. */
export interface AdvisorModelDto {
  name: string;
  format: string;
  bytes: number;
  host: string;
}

/** What went wrong on the graphics card: the helper process died, stopped answering, or the card reported an error. */
export type GpuProblemWhat = 'crash' | 'hang' | 'error';

/** The user's answer to a graphics card problem: never use the card again, or try it at the next load. */
export type GpuProblemChoice = 'useCpu' | 'tryAgain';

/** One graphics card incident. `id` is its Unix time in ms, unique; it names the incident when answering. */
export interface GpuProblemDto {
  id: number;
  /** Unix ms. */
  at: number;
  /** The driver's description of the card, when it is known. */
  card?: string;
  what: GpuProblemWhat;
}

export interface AdvisorStatusDto {
  /** The state of the model. */
  state: ModelState;
  model: AdvisorModelDto;
  options: AdvisorOptionsDto;
  download?: { received: number; total: number };
  /** Plain text: failed download, failed check, refused load. */
  problem?: string;
  /** This process's resident memory, while loaded. */
  ramBytes?: number;
  /** Memory of the machine. */
  ramTotal: number;
  /** This build can run the model on a graphics card; when false the switch for it is not shown. */
  gpuSupported: boolean;
  /** The operating system this app runs on: the wording (Recycle Bin or trash) and the path style follow it. */
  os: OsName;
  /** Where the loaded model runs, while loaded. */
  device?: DeviceDto;
  /** Video memory the model takes, while loaded on a graphics card. */
  videoBytes?: number;
  /** After the first generation. */
  tokensPerSecond?: number;
  labelling?: { scan: ScanId; done: number; total: number };
  /** An incident on the graphics card that the user has not answered yet; the job was finished on the processor. */
  gpuProblem?: GpuProblemDto;
  /** The last incident in the graphics card problem log, answered or not. */
  lastGpuProblem?: GpuProblemDto;
}

/** The model's verdict on a folder: never `safe`. Anything it does not call `keep` is `review`. */
export type ModelVerdict = 'review' | 'keep';

/** One folder the model labelled. `path` is the display path; `reason` is the model's text, shown as text only. */
export interface LabelDto {
  node: NodeId;
  path: string;
  allocated: number;
  verdict: ModelVerdict;
  reason: string;
  /** The user's own rule on this folder, when there is one (it wins over the label). */
  decided?: UserVerdict;
}

/** One step of "How a verdict is made": how many folders (and bytes) it handled, and one example. */
export interface StepDto {
  folders: number;
  allocated: number;
  example?: { path: string; verdict: Verdict };
}

export interface AdvisorLabelsDto {
  scan: ScanId;
  /** Folders the model labelled in this scan. */
  labelled: number;
  /** Folders a labelling run would take: no rule covers them, none holds a rule match, and the guard allows them. */
  candidates: number;
  /** Largest first. */
  labels: LabelDto[];
  /** How long the model ran, in milliseconds. */
  modelMs: number;
  /** Why the last run ended before all candidates were labelled. */
  stopped?: 'cancelled' | 'timeBudget';
  rules: StepDto;
  model: StepDto;
  guard: StepDto;
}

export interface AdvisorAnswerDto {
  node: NodeId;
  path: string;
  verdict: Verdict;
  /** Platter's own words, never the model's: the rule's description, why the guard leaves it alone, or that no rule covers it. */
  reason: string;
  /**
   * Who decided: the guard, a built-in rule, the user's own rule, the model (only the verdict is its label; the text
   * is Platter's), or nobody (`unruled`: no rule and no label, verdict `review`, "your call").
   */
  source: 'guard' | 'rule' | 'user' | 'model' | 'unruled';
  /** A short plain line: the rule's title, the folder's size and the share older than a year. */
  detail: string;
}

export type ScanIntent = 'largest' | 'cleanable' | 'labelled' | 'explain' | 'find' | 'general' | 'offtopic';
export type ScanKind = 'any' | 'cache' | 'build' | 'media';

/** One folder the answer is about, computed from the scan and the rules, never by the model. */
export interface ScanRowDto {
  node: NodeId;
  path: string;
  allocated: number;
  verdict?: Verdict;
  source?: 'guard' | 'rule' | 'user' | 'model';
  note?: string;
  /** The row is a file, not a folder. */
  file?: true;
}

/**
 * A piece of an answer: plain text, or a folder of the scan that the answer names (a row or the scope), bound to its
 * node with the verdict and source Platter itself gives that folder (never the model's; left out when nobody has an
 * opinion on it). `path` is the name as it is, a long one cut with `…` in the middle (the row has the whole path).
 */
export type AnswerSegmentDto =
  | { kind: 'text'; text: string }
  | {
      kind: 'path';
      node: NodeId;
      path: string;
      verdict?: Verdict;
      source?: 'guard' | 'rule' | 'user' | 'model';
    };

/** One way to read a question that could mean more than one thing. */
export interface ChoiceDto {
  /** What `pick` of the next `askScan` must be to take this choice. */
  id: number;
  label: string;
  /** The folder's display path, when the choice is one folder. */
  path?: string;
  node?: NodeId;
}

/** A question asked back: the words fit more than one folder or tool, and Platter does not guess. */
export interface ClarifyDto {
  prompt: string;
  choices: ChoiceDto[];
}

/** The answer to a free question: the text, how it was written, what was understood, and the rows it is about. */
export interface ScanAnswerDto {
  scan: ScanId;
  understood: {
    intent: ScanIntent;
    kind: ScanKind;
    limit: number;
    scope?: { node: NodeId; path: string; file?: true };
    scopeText?: string;
  };
  /** Text only; names in it are the folders' own and may hold any character. Every path of the scan is in “ ” quotes. */
  answer: string;
  /** The same answer in order: writing each text as it is and each path between “ ” gives `answer`. */
  segments: AnswerSegmentDto[];
  rows: ScanRowDto[];
  /** Matches that are not among the rows. */
  more: number;
  notes: string[];
  /**
   * Set when the question could mean more than one thing: `answer` is then the prompt, the rows and notes are empty,
   * and asking again with the `id` of one choice as `pick` gives the answer.
   */
  clarify?: ClarifyDto;
}

export interface FolderMatchDto {
  node: NodeId;
  path: string;
  allocated: number;
}

/** A folder or a regular file of a finished scan that a search found, for the autocomplete of the Ask card. */
export interface EntryMatchDto {
  node: NodeId;
  path: string;
  allocated: number;
  kind: 'dir' | 'file';
}

/** What the user says about a folder: always keep it, or it is safe to clean (the cleanup moves it to the trash). */
export type UserVerdict = 'keep' | 'safe';

/** One rule of the user's own. `path` is the folder for people to read (the home folder is `~`), never sent back. */
export interface UserRuleDto {
  id: number;
  path: string;
  verdict: UserVerdict;
}

/** The whole list of the user's rules, in the order they were made. */
export interface UserRulesDto {
  rules: UserRuleDto[];
  /**
   * Why the rules file cannot be used, in plain text. While it is set the list is empty, nothing can be added or
   * removed, and every cleanup plan is refused, until `resetUserRules` moves the file aside.
   */
  problem?: string;
}

// Errors are returned as `ApiError { code, message }` (prose in docs/architecture.md, "IPC contract").
export type ApiErrorCode = 'notFound' | 'cancelled' | 'io' | 'invalid';

export interface ApiError {
  code: ApiErrorCode;
  message: string;
}
