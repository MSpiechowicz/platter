import type {
  AdvisorLabelsDto,
  AdvisorStatusDto,
  LabelDto,
  ScanAnswerDto,
  ScanRowDto,
  StepDto,
} from '../../api/types';

export const MODEL_BYTES = 2_740_937_888;
export const GIB = 1024 ** 3;

/** A status of the model, by state, with the numbers the real one has. */
export function statusOf(
  state: AdvisorStatusDto['state'],
  extra: Partial<AdvisorStatusDto> = {},
): AdvisorStatusDto {
  return {
    state,
    model: {
      name: 'Qwen3.5-4B',
      format: 'GGUF · Q4_K_M',
      bytes: MODEL_BYTES,
      host: 'huggingface.co',
    },
    options: { enabled: true, unloadAfter: 'fiveMinutes', logQuestions: true },
    ramTotal: 32 * GIB,
    ...extra,
  };
}

export const step = (folders: number, extra: Partial<StepDto> = {}): StepDto => ({
  folders,
  allocated: folders * 1_000_000_000,
  ...extra,
});

export function label(node: number, gb: number, extra: Partial<LabelDto> = {}): LabelDto {
  return {
    node,
    path: `/home/demo/folder-${node}`,
    allocated: gb * 1_000_000_000,
    verdict: 'review',
    reason: `Reason of ${node}.`,
    ...extra,
  };
}

/** The counts and labels of a scan. */
export function labelsOf(extra: Partial<AdvisorLabelsDto> = {}): AdvisorLabelsDto {
  const labels = extra.labels ?? [];

  return {
    scan: 1,
    labelled: labels.length,
    candidates: 5,
    labels,
    modelMs: 4100,
    rules: step(1912, { example: { path: '/var/cache/pacman/pkg', verdict: 'safe' } }),
    model: step(labels.length),
    guard: step(3),
    ...extra,
  };
}

/**
 * A free-question answer about scan 1: two rows. The default text names the first row as a
 * path segment; an `answer` given without `segments` is one text segment, so a test of the text alone stays simple.
 */
export function scanAnswer(extra: Partial<ScanAnswerDto> = {}): ScanAnswerDto {
  const segments: ScanAnswerDto['segments'] =
    extra.answer === undefined
      ? [
          { kind: 'text', text: 'The biggest folder is ' },
          { kind: 'path', node: 11, path: '/home/demo/Videos', verdict: 'keep', source: 'guard' },
          { kind: 'text', text: ' (12 GB).' },
        ]
      : [{ kind: 'text', text: extra.answer }];

  return {
    scan: 1,
    understood: { intent: 'largest', kind: 'any', limit: 5 },
    answer: 'The biggest folder is \u201C/home/demo/Videos\u201D (12 GB).',
    segments,
    rows: [
      {
        node: 11,
        path: '/home/demo/Videos',
        allocated: 12_400_000_000,
        verdict: 'keep',
        source: 'guard',
        note: 'Your own files',
      },
      { node: 12, path: '/home/demo/.cache', allocated: 1_300_000_000, verdict: 'safe', source: 'rule' },
    ],
    more: 0,
    notes: [],
    ...extra,
  };
}

export function scanRow(node: number, extra: Partial<ScanRowDto> = {}): ScanRowDto {
  return { node, path: `/home/demo/folder-${node}`, allocated: 1_000_000_000, ...extra };
}
