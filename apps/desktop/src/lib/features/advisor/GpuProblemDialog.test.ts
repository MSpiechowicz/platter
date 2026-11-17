import { fireEvent, render, screen } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AdvisorStatusDto, GpuProblemDto } from '../../api/types';
import { advisor } from '../../state/advisor.svelte';
import { settings } from '../../state/settings.svelte';
import { statusOf } from './fixtures';
import GpuProblemDialog from './GpuProblemDialog.svelte';

const { api, calls } = vi.hoisted(() => ({
  calls: [] as string[],
  api: {
    answerGpuProblem: vi.fn(),
    configureAdvisor: vi.fn(),
  },
}));

vi.mock('../../api', () => ({ api }));

const CARD = 'AMD Radeon RX 7900 XTX (RADV NAVI31)';
const SWITCHED = ' Platter stopped using the card for now. The model runs on the processor until you choose.';

const problem = (extra: Partial<GpuProblemDto> = {}): GpuProblemDto => ({
  id: 1_760_000_000_000,
  at: 1_760_000_000_000,
  card: CARD,
  what: 'crash',
  ...extra,
});

const withProblem = (gpuProblem: GpuProblemDto | undefined): AdvisorStatusDto =>
  statusOf('loaded', {
    device: { kind: 'cpu' },
    ...(gpuProblem ? { gpuProblem, lastGpuProblem: gpuProblem } : {}),
  });

const dialog = (): HTMLDialogElement => document.querySelector('dialog') as HTMLDialogElement;
const tryAgain = (): HTMLElement => screen.getByRole('button', { name: 'Try the card again next time' });
const useCpu = (): HTMLElement => screen.getByRole('button', { name: 'Use the processor from now on' });

function show(gpuProblem: GpuProblemDto | null): void {
  advisor.status = withProblem(gpuProblem ?? undefined);
  render(GpuProblemDialog);
  flushSync();
}

describe('GpuProblemDialog', () => {
  beforeEach(() => {
    calls.length = 0;
    api.answerGpuProblem.mockReset().mockImplementation(async () => {
      calls.push('answer');

      return withProblem(undefined);
    });
    api.configureAdvisor.mockReset().mockImplementation(async (options) => {
      calls.push(`configure:${options.useGpu}`);

      return statusOf('loaded', { options });
    });
    advisor.status = undefined;
    advisor.failure = undefined;
    // A problem closed unanswered is remembered for the session: use ids no other test used.
    settings.set('advisorUseGpu', true);
    settings.set('advisorEnabled', true);
  });

  it('opens with the title, says what happened on which card, and focuses "Use the processor from now on"', async () => {
    show(problem({ id: 11 }));
    await tick();

    expect(dialog().hasAttribute('open')).toBe(true);
    expect(screen.getByRole('heading', { name: 'The graphics card stopped working' })).toBeTruthy();
    expect(screen.getByText(`The advisor crashed on AMD Radeon RX 7900 XTX.${SWITCHED}`)).toBeTruthy();
    expect(document.activeElement).toBe(useCpu());
    expect(useCpu().classList.contains('primary')).toBe(true);
  });

  it('says the advisor stopped responding for a hang', () => {
    show(problem({ id: 12, what: 'hang' }));

    expect(
      screen.getByText(`The advisor stopped responding on AMD Radeon RX 7900 XTX.${SWITCHED}`),
    ).toBeTruthy();
  });

  it('says the card reported an error, with its name in brackets', () => {
    show(problem({ id: 13, what: 'error' }));

    expect(
      screen.getByText(`The graphics card reported an error (AMD Radeon RX 7900 XTX).${SWITCHED}`),
    ).toBeTruthy();
  });

  it('names the card as "the graphics card" when its name is not known', () => {
    show(problem({ id: 14, card: undefined, what: 'hang' }));
    expect(screen.getByText(`The advisor stopped responding on the graphics card.${SWITCHED}`)).toBeTruthy();
  });

  it('does not name the card twice for an error on an unknown card', () => {
    show(problem({ id: 15, card: undefined, what: 'error' }));
    expect(screen.getByText(`The graphics card reported an error.${SWITCHED}`)).toBeTruthy();
  });

  it('keeps the whole name when only the driver part would be left, and makes hidden characters in it visible', () => {
    show(problem({ id: 16, card: '(RADV NAVI31)' }));
    expect(screen.getByText(`The advisor crashed on (RADV NAVI31).${SWITCHED}`)).toBeTruthy();
  });

  it('shows hidden characters of the card name as text', () => {
    show(problem({ id: 17, card: 'Card‮X (RADV)' }));
    expect(screen.getByText(`The advisor crashed on Card\\u{202E}X.${SWITCHED}`)).toBeTruthy();
  });

  it('is closed while no problem waits', () => {
    show(null);

    expect(dialog().hasAttribute('open')).toBe(false);
    expect(screen.queryByRole('button', { name: 'Try the card again next time' })).toBeNull();
  });

  it('"Use the processor from now on" answers useCpu, then saves the card off and configures', async () => {
    show(problem({ id: 21 }));

    await fireEvent.click(useCpu());

    expect(api.answerGpuProblem).toHaveBeenCalledExactlyOnceWith(21, 'useCpu');
    expect(calls).toEqual(['answer', 'configure:false']);
    expect(settings.advisorUseGpu).toBe(false);
    expect(dialog().hasAttribute('open')).toBe(false);
  });

  it('"Try the card again next time" answers tryAgain and changes nothing else', async () => {
    show(problem({ id: 22 }));

    await fireEvent.click(tryAgain());

    expect(api.answerGpuProblem).toHaveBeenCalledExactlyOnceWith(22, 'tryAgain');
    expect(api.configureAdvisor).not.toHaveBeenCalled();
    expect(settings.advisorUseGpu).toBe(true);
    expect(dialog().hasAttribute('open')).toBe(false);
  });

  it('answers nothing on Escape, and does not open again this session', async () => {
    show(problem({ id: 31 }));

    // What Escape does in a browser: the dialog closes itself and then fires `close`.
    dialog().close();
    await tick();

    expect(api.answerGpuProblem).not.toHaveBeenCalled();
    expect(api.configureAdvisor).not.toHaveBeenCalled();
    expect(settings.advisorUseGpu).toBe(true);
    expect(dialog().hasAttribute('open')).toBe(false);

    // The same problem comes in again with the next status: it stays closed.
    advisor.status = withProblem(problem({ id: 31 }));
    flushSync();

    expect(dialog().hasAttribute('open')).toBe(false);
  });

  it('answers nothing when the backdrop is clicked, and opens again for a new problem', async () => {
    show(problem({ id: 41 }));

    await fireEvent.click(dialog());

    expect(api.answerGpuProblem).not.toHaveBeenCalled();
    expect(dialog().hasAttribute('open')).toBe(false);

    advisor.status = withProblem(problem({ id: 42, what: 'hang' }));
    flushSync();
    await tick();

    expect(dialog().hasAttribute('open')).toBe(true);
    expect(screen.getByText(/The advisor stopped responding on/)).toBeTruthy();
  });
});
