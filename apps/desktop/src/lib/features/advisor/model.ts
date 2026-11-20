import type {
  AdvisorAnswerDto,
  AdvisorLabelsDto,
  AdvisorStatusDto,
  GpuProblemDto,
  GpuProblemWhat,
  LabelDto,
  ModelState,
  StepDto,
  UnloadAfter,
  Verdict,
} from '../../api/types';
import { formatDateTime } from '../../format';

/** Formats bytes the way the rest of the interface does (it follows the size-unit setting). */
export type SizeFormat = (bytes: number) => string;

/** What the state pill says. `undefined` is the moment before the backend has answered. */
export function pillText(state: ModelState | undefined): string {
  switch (state) {
    case 'loaded':
      return 'Model loaded';
    case 'onDisk':
      return 'Model on disk, not loaded';
    case 'notDownloaded':
      return 'Not downloaded';
    case 'downloading':
      return 'Downloading…';
    case 'loading':
      return 'Loading model…';
    case 'unavailable':
      return 'Not available here';
    default:
      return 'Checking the model…';
  }
}

/** The text for a download that has bytes: "512 MB of 2.7 GB". */
export function receivedText(download: { received: number; total: number }, size: SizeFormat): string {
  return `${size(download.received)} of ${size(download.total)}`;
}

/** How much of the download has arrived, 0 to 1. */
export function downloadShare(download: { received: number; total: number }): number {
  if (download.total <= 0) {
    return 0;
  }

  return Math.max(0, Math.min(1, download.received / download.total));
}

/** The label of the download button: a cancelled download is continued, not started again. */
export function downloadLabel(status: AdvisorStatusDto, size: SizeFormat): string {
  if (status.download) {
    return `Continue download (${receivedText(status.download, size)})`;
  }

  return `Download ${status.model.name} (${size(status.model.bytes)})`;
}

export type PrimaryAction =
  | { kind: 'download'; label: string }
  | { kind: 'cancel'; label: string }
  | { kind: 'load'; label: string; disabled: boolean; hint?: string }
  | { kind: 'unload'; label: string }
  | { kind: 'busy'; label: string };

/** What a card about a scan says while there is no finished scan: nothing was scanned, or the scan is still running. */
export function noScanText(scanning: boolean): string {
  return scanning
    ? 'A scan is running. The advisor uses it once it finishes.'
    : 'Scan a disk or folder first.';
}

export const SWITCHED_OFF_HINT = 'The local model is switched off in Settings.';

/** The one button of the page header, by the model's state. Nothing for a build without a runtime. */
export function primaryAction(
  status: AdvisorStatusDto | undefined,
  enabled: boolean,
  size: SizeFormat,
): PrimaryAction | undefined {
  switch (status?.state) {
    case 'notDownloaded':
      // The Model card right below names the model; the header keeps the button short so it ends where the cards do.
      return {
        kind: 'download',
        label: status.download ? downloadLabel(status, size) : `Download (${size(status.model.bytes)})`,
      };
    case 'downloading':
      return { kind: 'cancel', label: 'Cancel download' };
    case 'onDisk':
      return {
        kind: 'load',
        label: 'Load model',
        disabled: !enabled,
        hint: enabled ? undefined : SWITCHED_OFF_HINT,
      };
    case 'loaded':
      return { kind: 'unload', label: 'Unload model' };
    case 'loading':
      // The pill beside it says the model is loading; the button keeps its place and its name.
      return { kind: 'busy', label: 'Load model' };
    default:
      return undefined;
  }
}

/** The text of the "Unloads after" row. */
export function unloadAfterText(unloadAfter: UnloadAfter): string {
  switch (unloadAfter) {
    case 'oneMinute':
      return '1 min idle';
    case 'fiveMinutes':
      return '5 min idle';
    case 'windowClose':
      return 'When the window closes';
  }
}

/** The options of the Unload dropdown in Settings. */
export const UNLOAD_OPTIONS: { value: UnloadAfter; label: string }[] = [
  { value: 'oneMinute', label: '1 minute idle' },
  { value: 'fiveMinutes', label: '5 minutes idle' },
  { value: 'windowClose', label: 'When the window closes' },
];

/** "36 tokens/s", or a dash before the first answer. */
export function speedText(tokensPerSecond: number | undefined): string {
  if (tokensPerSecond === undefined || !Number.isFinite(tokensPerSecond)) {
    return '—';
  }

  return `${Math.round(tokensPerSecond)} tokens/s`;
}

/** How much of the machine's memory the model holds, 0 to 1. */
export function ramShare(status: AdvisorStatusDto): number {
  if (status.ramBytes === undefined || status.ramTotal <= 0) {
    return 0;
  }

  return Math.max(0, Math.min(1, status.ramBytes / status.ramTotal));
}

/** "2.1 GB of 34 GB RAM in use"; undefined while the model is not in memory. */
export function ramText(status: AdvisorStatusDto, size: SizeFormat): string | undefined {
  if (status.ramBytes === undefined) {
    return undefined;
  }

  return `${size(status.ramBytes)} of ${size(status.ramTotal)} RAM in use`;
}

/**
 * "GPU · AMD Radeon RX 7900 XTX (RADV NAVI31)" or "CPU" for the "Runs on" row; a dash while the model is not in
 * memory. `short` leaves off the driver's closing "(…)" so the card's name fits the row.
 */
export function deviceText(status: AdvisorStatusDto, short = false): string {
  if (status.state !== 'loaded' || !status.device) {
    return '—';
  }

  if (status.device.kind === 'cpu') {
    return 'CPU';
  }

  if (!status.device.name) {
    return 'GPU';
  }

  return `GPU · ${short ? shortCardName(status.device.name) : status.device.name}`;
}

/** The card's name without the driver's closing "(RADV NAVI31)"; the whole name when nothing else is left. */
export function shortCardName(name: string): string {
  const trimmed = name.trimEnd();
  if (!trimmed.endsWith(')')) {
    return name;
  }

  // The last "(" is the only one that can open the closing group, and nothing inside it may close or open.
  const open = trimmed.lastIndexOf('(');
  if (open === -1 || trimmed.indexOf(')', open) !== trimmed.length - 1) {
    return name;
  }

  return trimmed.slice(0, open).trimEnd() || name;
}

/** What the dialog says about a graphics card problem, to be shown with hidden characters made visible. */
export function gpuProblemText(problem: GpuProblemDto): string {
  const card = problem.card === undefined ? undefined : shortCardName(problem.card);
  const where = card ?? 'the graphics card';

  const what = {
    crash: `The advisor crashed on ${where}.`,
    hang: `The advisor stopped responding on ${where}.`,
    // Without a name the card is already named: "(the graphics card)" would say it twice.
    error:
      card === undefined
        ? 'The graphics card reported an error.'
        : `The graphics card reported an error (${card}).`,
  }[problem.what];

  return `${what} Platter stopped using the card for now. The model runs on the processor until you choose.`;
}

const GPU_PROBLEM_WORDS: Record<GpuProblemWhat, string> = {
  crash: 'crashed',
  hang: 'stopped responding',
  error: 'error',
};

/** "12 Mar 2025, 14:05 · crashed": the value of the "Last graphics card problem" row. */
export function gpuProblemSummary(problem: GpuProblemDto): string {
  return `${formatDateTime(problem.at / 1000)} · ${GPU_PROBLEM_WORDS[problem.what]}`;
}

/** "3.2 GB video memory in use"; undefined unless the model is loaded on a graphics card. */
export function videoText(status: AdvisorStatusDto, size: SizeFormat): string | undefined {
  if (status.device?.kind !== 'gpu' || status.videoBytes === undefined) {
    return undefined;
  }

  return `${size(status.videoBytes)} video memory in use`;
}

/** The "On disk" row: the size once the file is there. */
export function onDiskText(status: AdvisorStatusDto, size: SizeFormat): string {
  switch (status.state) {
    case 'onDisk':
    case 'loading':
    case 'loaded':
      return size(status.model.bytes);
    case 'downloading':
      return 'Downloading…';
    default:
      return 'Not downloaded';
  }
}

/** One part of the bar under the three steps. `share` is 0 to 1; the parts add up to 1. */
export interface SplitPart {
  key: 'rules' | 'model' | 'guard';
  folders: number;
  share: number;
}

/** The folders each step handled, as parts of all of them (the mockup's bar counts folders). */
export function splitParts(labels: AdvisorLabelsDto): SplitPart[] {
  const steps: [SplitPart['key'], StepDto][] = [
    ['rules', labels.rules],
    ['model', labels.model],
    ['guard', labels.guard],
  ];
  const total = steps.reduce((sum, [, step]) => sum + step.folders, 0);

  return steps
    .filter(([, step]) => step.folders > 0)
    .map(([key, step]) => ({ key, folders: step.folders, share: step.folders / total }));
}

/** The legend's percentage is the app's one rule: see `percentText` in `format.ts`. */
export { percentText } from '../../format';

/**
 * The labels the user has to look at: the model says `review`. One the user has decided already (`decided`) stays in
 * the list and shows that decision in place of the buttons, so a decision is seen and the list does not jump.
 */
export function needsInput(labels: AdvisorLabelsDto | undefined): LabelDto[] {
  return (labels?.labels ?? []).filter((label) => label.verdict === 'review');
}

/** The name of whoever decided, for the verdict line of an answer. */
export function sourceLabel(source: AdvisorAnswerDto['source']): string {
  switch (source) {
    case 'guard':
      return 'Safety guard';
    case 'rule':
      return 'Built-in rule';
    case 'user':
      return 'Your rule';
    case 'model':
      return 'Local model';
    case 'unruled':
      return 'No rule';
  }
}

export interface LabelReadiness {
  /** "Label folders" can be pressed. */
  ready: boolean;
  /** Why it cannot be, as one plain sentence. */
  reason?: string;
}

/** What stands between the user and "Label folders" in a finished scan. */
export function labelReadiness(input: {
  status: AdvisorStatusDto | undefined;
  enabled: boolean;
  outdated: boolean;
  labels: AdvisorLabelsDto | undefined;
}): LabelReadiness {
  const { status, enabled, outdated, labels } = input;
  const no = (reason: string): LabelReadiness => ({ ready: false, reason });

  if (!status) {
    return no('Checking the model…');
  }

  if (status.state === 'unavailable') {
    return no('The local model is not available in this build.');
  }

  if (status.state === 'notDownloaded' || status.state === 'downloading') {
    return no('Download the model first.');
  }

  if (!enabled) {
    return no(SWITCHED_OFF_HINT);
  }

  // The page header's pill says the model is loading; the card does not say it again.
  if (status.state === 'loading') {
    return { ready: false };
  }

  if (outdated) {
    return no('Labelling is paused until you rescan.');
  }

  if (labels && labels.candidates === 0) {
    return no('No folder is left for the model: the rules cover the rest.');
  }

  return { ready: true };
}

/** "3 of 50 folders". */
export function progressText(done: number, total: number): string {
  return `${done} of ${total} folders`;
}

/** What the question box is not ready for, as one plain sentence; `ready` when a question can be asked. */
export interface AskReadiness {
  ready: boolean;
  reason?: string;
}

/**
 * What stands between the user and a free question. The backend reads the question with the model, so the model must
 * be there and switched on, and it is busy while it labels. The page is about a finished scan.
 */
export function askReadiness(input: {
  hasScan: boolean;
  scanning: boolean;
  status: AdvisorStatusDto | undefined;
  enabled: boolean;
}): AskReadiness {
  const { hasScan, scanning, status, enabled } = input;
  const no = (reason: string): AskReadiness => ({ ready: false, reason });

  if (!hasScan) {
    return no(noScanText(scanning));
  }

  if (!status) {
    return no('Checking the model…');
  }

  if (status.state === 'unavailable') {
    return no('The local model is not available in this build.');
  }

  if (status.state === 'notDownloaded' || status.state === 'downloading') {
    return no('Download the model first.');
  }

  if (!enabled) {
    return no(SWITCHED_OFF_HINT);
  }

  // The page header's pill says the model is loading; the card does not say it again.
  if (status.state === 'loading') {
    return { ready: false };
  }

  if (status.labelling) {
    return no('The model is labelling folders, so ask again when it is done.');
  }

  return { ready: true };
}

/** What a folder tag in an answer says of its verdict, for the name a screen reader reads ("no verdict" when none). */
export function verdictWords(verdict: Verdict | undefined): string {
  switch (verdict) {
    case 'safe':
      return 'safe to clean';
    case 'review':
      return 'needs review';
    case 'keep':
      return 'kept';
    case undefined:
      return 'no verdict';
  }
}
