import { describe, expect, it } from 'vitest';
import type { ModelState } from '../../api/types';
import { formatSize } from '../../format';
import { GIB, MODEL_BYTES, label, labelsOf, statusOf } from './fixtures';
import {
  askReadiness,
  deviceText,
  downloadLabel,
  downloadShare,
  labelReadiness,
  needsInput,
  onDiskText,
  percentText,
  pillText,
  primaryAction,
  progressText,
  ramShare,
  ramText,
  receivedText,
  shortCardName,
  sourceLabel,
  speedText,
  splitParts,
  unloadAfterText,
  videoText,
} from './model';

const size = (bytes: number): string => formatSize(bytes, 'GB');

describe('pillText', () => {
  it('names the state of the model', () => {
    const texts: [ModelState | undefined, string][] = [
      ['loaded', 'Model loaded'],
      ['onDisk', 'Model on disk, not loaded'],
      ['notDownloaded', 'Not downloaded'],
      ['downloading', 'Downloading…'],
      ['loading', 'Loading model…'],
      ['unavailable', 'Not available here'],
      [undefined, 'Checking the model…'],
    ];

    for (const [state, text] of texts) {
      expect(pillText(state)).toBe(text);
    }
  });
});

describe('download texts', () => {
  it('names the model and its size at first and "Continue download" with what is there after a cancel', () => {
    expect(downloadLabel(statusOf('notDownloaded'), size)).toBe('Download Qwen3.5-4B (2.7 GB)');
    expect(
      downloadLabel(
        statusOf('notDownloaded', { download: { received: 512_000_000, total: MODEL_BYTES } }),
        size,
      ),
    ).toBe('Continue download (512 MB of 2.7 GB)');
  });

  it('measures the share received, never outside 0 to 1', () => {
    expect(downloadShare({ received: 685_234_472, total: MODEL_BYTES })).toBeCloseTo(0.25, 5);
    expect(downloadShare({ received: 5, total: 0 })).toBe(0);
    expect(downloadShare({ received: MODEL_BYTES * 2, total: MODEL_BYTES })).toBe(1);
    expect(receivedText({ received: 400_000_000, total: MODEL_BYTES }, size)).toBe('400 MB of 2.7 GB');
  });
});

describe('primaryAction', () => {
  it('offers the one fitting action of each state', () => {
    expect(primaryAction(statusOf('notDownloaded'), false, size)).toEqual({
      kind: 'download',
      label: 'Download (2.7 GB)',
    });
    expect(primaryAction(statusOf('downloading'), true, size)).toEqual({
      kind: 'cancel',
      label: 'Cancel download',
    });
    expect(primaryAction(statusOf('onDisk'), true, size)).toEqual({
      kind: 'load',
      label: 'Load model',
      disabled: false,
      hint: undefined,
    });
    expect(primaryAction(statusOf('loaded'), true, size)).toEqual({
      kind: 'unload',
      label: 'Unload model',
    });
    expect(primaryAction(statusOf('loading'), true, size)).toEqual({ kind: 'busy', label: 'Load model' });
  });

  it('has no action before the status is known or where there is no runtime', () => {
    expect(primaryAction(undefined, true, size)).toBeUndefined();
    expect(primaryAction(statusOf('unavailable'), true, size)).toBeUndefined();
  });

  it('keeps Load off, and says why, while the model is switched off in the settings', () => {
    expect(primaryAction(statusOf('onDisk'), false, size)).toMatchObject({
      kind: 'load',
      disabled: true,
      hint: 'The local model is switched off in Settings.',
    });
  });
});

describe('the model card figures', () => {
  it('shows the speed once it is known, and a dash before', () => {
    expect(speedText(36.5)).toBe('37 tokens/s');
    expect(speedText(undefined)).toBe('—');
    expect(speedText(Number.NaN)).toBe('—');
  });

  it('says when the model unloads', () => {
    expect(unloadAfterText('oneMinute')).toBe('1 min idle');
    expect(unloadAfterText('fiveMinutes')).toBe('5 min idle');
    expect(unloadAfterText('windowClose')).toBe('When the window closes');
  });

  it('reports RAM only while the model is in memory', () => {
    const loaded = statusOf('loaded', { ramBytes: 2 * GIB });

    expect(ramText(loaded, size)).toBe('2.1 GB of 34 GB RAM in use');
    expect(ramShare(loaded)).toBeCloseTo(2 / 32, 5);
    expect(ramText(statusOf('onDisk'), size)).toBeUndefined();
    expect(ramShare(statusOf('onDisk'))).toBe(0);
    expect(ramShare(statusOf('loaded', { ramBytes: 64 * GIB }))).toBe(1);
  });

  it('says where the loaded model runs, and a dash while it is not in memory', () => {
    const card = { kind: 'gpu', name: 'AMD Radeon RX 7900 XTX (RADV NAVI31)' } as const;

    expect(deviceText(statusOf('loaded', { device: card }))).toBe(
      'GPU · AMD Radeon RX 7900 XTX (RADV NAVI31)',
    );
    expect(deviceText(statusOf('loaded', { device: card }), true)).toBe('GPU · AMD Radeon RX 7900 XTX');
    expect(deviceText(statusOf('loaded', { device: { kind: 'gpu', name: '(Card)' } }), true)).toBe(
      'GPU · (Card)',
    );
    expect(deviceText(statusOf('loaded', { device: { kind: 'gpu' } }))).toBe('GPU');
    expect(deviceText(statusOf('loaded', { device: { kind: 'cpu' } }))).toBe('CPU');
    expect(deviceText(statusOf('loaded'))).toBe('—');
    expect(deviceText(statusOf('onDisk', { device: card }))).toBe('—');
  });

  it('reports video memory only while the model is loaded on a graphics card', () => {
    const onCard = statusOf('loaded', { device: { kind: 'gpu', name: 'Card' }, videoBytes: 3_200_000_000 });

    expect(videoText(onCard, size)).toBe('3.2 GB video memory in use');
    expect(
      videoText(statusOf('loaded', { device: { kind: 'cpu' }, videoBytes: 3_200_000_000 }), size),
    ).toBeUndefined();
    expect(videoText(statusOf('loaded', { device: { kind: 'gpu' } }), size)).toBeUndefined();
    expect(videoText(statusOf('onDisk'), size)).toBeUndefined();
  });

  it('shows the size on disk once the file is there', () => {
    expect(onDiskText(statusOf('onDisk'), size)).toBe('2.7 GB');
    expect(onDiskText(statusOf('loaded'), size)).toBe('2.7 GB');
    expect(onDiskText(statusOf('notDownloaded'), size)).toBe('Not downloaded');
    expect(
      onDiskText(statusOf('downloading', { download: { received: 400_000_000, total: MODEL_BYTES } }), size),
    ).toBe('Downloading…');
  });
});

describe('splitParts', () => {
  it('splits the folders of the steps, and leaves out a step that handled none', () => {
    const parts = splitParts(labelsOf({ labels: [label(1, 20)] }));

    expect(parts.map((part) => part.key)).toEqual(['rules', 'model', 'guard']);
    expect(parts.reduce((sum, part) => sum + part.share, 0)).toBeCloseTo(1, 10);
    expect(percentText(parts[0]?.share ?? 0)).toBe('99%');

    expect(percentText(1)).toBe('100%');
    expect(percentText(0.001)).toBe('<1%');
    expect(percentText(0.875)).toBe('88%');

    const noModel = splitParts(labelsOf());

    expect(noModel.map((part) => part.key)).toEqual(['rules', 'guard']);
  });

  it('has no part when nothing was handled', () => {
    const empty = labelsOf({ rules: { folders: 0, allocated: 0 }, guard: { folders: 0, allocated: 0 } });

    expect(splitParts(empty)).toEqual([]);
  });
});

describe('needsInput', () => {
  it('lists the folders the model wants reviewed, those the user decided too, not the ones it keeps', () => {
    const labels = labelsOf({
      labels: [
        label(1, 30),
        label(2, 20, { verdict: 'keep' }),
        label(3, 10, { decided: 'keep' }),
        label(4, 5),
      ],
    });

    expect(needsInput(labels).map((item) => item.node)).toEqual([1, 3, 4]);
    expect(needsInput(undefined)).toEqual([]);
  });
});

describe('sourceLabel', () => {
  it('names who decided', () => {
    expect(sourceLabel('guard')).toBe('Safety guard');
    expect(sourceLabel('rule')).toBe('Built-in rule');
    expect(sourceLabel('user')).toBe('Your rule');
    expect(sourceLabel('model')).toBe('Local model');
    expect(sourceLabel('unruled')).toBe('No rule');
  });
});

describe('labelReadiness', () => {
  const base = { status: statusOf('loaded'), enabled: true, outdated: false, labels: labelsOf() };

  it('is ready with a model that can run, a current scan and folders to label', () => {
    expect(labelReadiness(base)).toEqual({ ready: true });
    expect(labelReadiness({ ...base, status: statusOf('onDisk') })).toEqual({ ready: true });
  });

  it('says what stands in the way, in the order the user would fix it', () => {
    const reason = (input: Partial<typeof base>): string | undefined =>
      labelReadiness({ ...base, ...input }).reason;

    expect(reason({ status: undefined })).toBe('Checking the model…');
    expect(reason({ status: statusOf('unavailable') })).toBe(
      'The local model is not available in this build.',
    );
    expect(reason({ status: statusOf('notDownloaded') })).toBe('Download the model first.');
    expect(reason({ status: statusOf('downloading') })).toBe('Download the model first.');
    expect(reason({ enabled: false })).toBe('The local model is switched off in Settings.');
    expect(reason({ status: statusOf('loading') })).toBeUndefined();
    expect(reason({ outdated: true })).toBe('Rescan first: a cleanup changed the disk.');
    expect(reason({ labels: labelsOf({ candidates: 0 }) })).toBe(
      'No folder is left for the model: the rules cover the rest.',
    );
  });
});

describe('progressText', () => {
  it('reads "N of M folders"', () => {
    expect(progressText(3, 50)).toBe('3 of 50 folders');
  });
});

describe('askReadiness', () => {
  const base = { hasScan: true, scanning: false, status: statusOf('onDisk'), enabled: true };

  it('is ready with a scan and a model that is there and switched on', () => {
    expect(askReadiness(base)).toEqual({ ready: true });
    expect(askReadiness({ ...base, status: statusOf('loaded') })).toEqual({ ready: true });
  });

  it('says what stands in the way', () => {
    const reason = (input: Partial<typeof base>): string | undefined =>
      askReadiness({ ...base, ...input }).reason;

    expect(reason({ hasScan: false })).toBe('Scan a disk or folder first.');
    expect(reason({ hasScan: false, scanning: true })).toBe(
      'A scan is running. The advisor uses it once it finishes.',
    );
    expect(reason({ status: undefined })).toBe('Checking the model…');
    expect(reason({ status: statusOf('notDownloaded') })).toBe('Download the model first.');
    expect(reason({ enabled: false })).toBe('The local model is switched off in Settings.');
    expect(reason({ status: statusOf('loading') })).toBeUndefined();
    expect(reason({ status: statusOf('loaded', { labelling: { scan: 1, done: 1, total: 4 } }) })).toMatch(
      /labelling/,
    );
  });
});

describe('shortCardName', () => {
  it('drops the closing group of the driver and keeps the rest', () => {
    expect(shortCardName('AMD Radeon RX 7900 XTX (RADV NAVI31)')).toBe('AMD Radeon RX 7900 XTX');
    expect(shortCardName('Card   (RADV)  ')).toBe('Card');
    expect(shortCardName('Card (a) (b)')).toBe('Card (a)');
  });

  it('keeps the whole name when nothing else is left or the group is not a plain closing one', () => {
    expect(shortCardName('(RADV NAVI31)')).toBe('(RADV NAVI31)');
    expect(shortCardName('  (RADV)')).toBe('  (RADV)');
    expect(shortCardName('Card')).toBe('Card');
    expect(shortCardName('Card (a (b))')).toBe('Card (a (b))');
    expect(shortCardName('Card (a) b)')).toBe('Card (a) b)');
    expect(shortCardName('Card a)')).toBe('Card a)');
    expect(shortCardName('Card (a) b')).toBe('Card (a) b');
  });

  it('is quick on a long run of spaces and on a long name', () => {
    const started = performance.now();

    expect(shortCardName(' '.repeat(1e5) + 'x')).toBe(' '.repeat(1e5) + 'x');
    expect(shortCardName(`${'x'.repeat(1e5)} (RADV)`)).toBe('x'.repeat(1e5));
    expect(shortCardName('('.repeat(1e5) + ')')).toBe('('.repeat(1e5 - 1));
    expect(performance.now() - started).toBeLessThan(50);
  });
});
