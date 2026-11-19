import { render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { LAND_EASING, LAND_MS, opening, STEP_MIN_MS } from '../../state/opening.svelte';
import { settings } from '../../state/settings.svelte';
import OpeningScreen from './OpeningScreen.svelte';

const NOW = 1_750_000_000;

const rect = (left: number, top: number, width: number): DOMRect =>
  ({ left, top, width, height: width, right: left + width, bottom: top + width, x: left, y: top }) as DOMRect;

describe('OpeningScreen', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(NOW * 1000));
    settings.set('units', 'GB');
    settings.reduceMotion = false;
    opening.reset('opening');
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('says what it is doing and lists the three steps, all waiting', () => {
    render(OpeningScreen, { props: { target: null } });

    expect(screen.getByRole('heading', { name: 'Opening Platter' })).toBeTruthy();
    expect(screen.getByText('Getting your disks and your last scan ready')).toBeTruthy();

    const items = screen.getAllByRole('listitem');

    expect(items.map((item) => item.querySelector('.label')?.textContent)).toEqual([
      'Checking your disks',
      'Looking for your last scan',
      'Checking it is still current',
    ]);
    expect(items.every((item) => item.classList.contains('waiting'))).toBe(true);
  });

  it('shows the step that is running, then its note when it is done', async () => {
    render(OpeningScreen, { props: { target: null } });

    const running = opening.run({
      disks: async () => 3,
      lastScan: async () => ({ bytes: 983_000_000_000, scannedAt: NOW - 7200, outdated: true }),
    });
    const item = (index: number): HTMLElement => screen.getAllByRole('listitem')[index] as HTMLElement;

    await vi.advanceTimersByTimeAsync(0);
    expect(item(0).classList.contains('running')).toBe(true);
    // A screen reader hears the step in progress, and then what each one found.
    expect(screen.getByRole('status').textContent).toBe('Checking your disks…');

    await vi.advanceTimersByTimeAsync(STEP_MIN_MS);
    expect(item(0).classList.contains('done')).toBe(true);
    expect(screen.getByText('3 disks')).toBeTruthy();
    expect(item(1).classList.contains('running')).toBe(true);
    expect(screen.getByRole('status').textContent).toBe('Looking for your last scan…');

    await vi.advanceTimersByTimeAsync(2 * STEP_MIN_MS);
    expect(screen.getByText('983 GB · 2 hours ago')).toBeTruthy();

    // An outdated scan is flagged in amber.
    expect(item(2).classList.contains('warn')).toBe(true);
    expect(screen.getByText('outdated, rescan to refresh')).toBeTruthy();
    expect(screen.getByRole('status').textContent).toBe(
      'Checking it is still current: outdated, rescan to refresh',
    );

    await vi.advanceTimersByTimeAsync(5000);
    await running;
  });

  it('shows the last step skipped, with no note, when there is no saved scan', async () => {
    render(OpeningScreen, { props: { target: null } });

    const running = opening.run({ disks: async () => 1, lastScan: async () => null });

    await vi.advanceTimersByTimeAsync(2 * STEP_MIN_MS);

    expect(screen.getByText('1 disk')).toBeTruthy();
    expect(screen.getByText('none kept')).toBeTruthy();
    expect(screen.getAllByRole('listitem')[2]?.classList.contains('skipped')).toBe(true);

    await vi.advanceTimersByTimeAsync(5000);
    await running;
  });

  describe('landing', () => {
    // The shell, the menu (which is sliding in, so it is 220 px to the left) and the logo inside it.
    function menu() {
      const shell = document.createElement('div');
      const side = document.createElement('aside');
      const logo = document.createElement('div');

      side.append(logo);
      shell.append(side);
      document.body.append(shell);
      vi.spyOn(shell, 'getBoundingClientRect').mockReturnValue(rect(0, 0, 1000));
      vi.spyOn(side, 'getBoundingClientRect').mockReturnValue(rect(-220, 0, 220));
      vi.spyOn(logo, 'getBoundingClientRect').mockReturnValue(rect(-220 + 17, 22, 26));

      return { logo, shell };
    }

    function flights() {
      const finish: { run?: () => void } = {};
      const animate = vi.fn((_keyframes: Keyframe[], _options: KeyframeAnimationOptions) => ({
        cancel: () => {},
        set onfinish(callback: (() => void) | null) {
          finish.run = callback ?? undefined;
        },
      }));

      vi.spyOn(Element.prototype, 'animate').mockImplementation(animate as never);

      return { animate, finish };
    }

    it('flies the logo onto the one in the menu, wherever the sliding menu is, and ends when it arrives', async () => {
      const { logo, shell } = menu();
      const { animate, finish } = flights();

      opening.reset('landing');
      vi.spyOn(SVGElement.prototype, 'getBoundingClientRect').mockReturnValue(rect(340, 200, 120));
      render(OpeningScreen, { props: { target: logo } });
      await vi.advanceTimersByTimeAsync(0);

      expect(animate).toHaveBeenCalledTimes(1);

      const [keyframes, options] = animate.mock.calls[0] as [Keyframe[], KeyframeAnimationOptions];

      // The menu rests at the left edge, so its logo rests 17 px in, whatever the slide has done so far.
      expect(keyframes.at(-1)?.transform).toBe(`translate(${17 - 340}px, ${22 - 200}px) scale(${26 / 120})`);
      expect(options.duration).toBe(LAND_MS);
      expect(options.easing).toBe(LAND_EASING);
      expect(opening.phase).toBe('landing');

      finish.run?.();
      expect(opening.phase).toBe('done');

      shell.remove();
    });

    it('is over at once when there is no logo in the menu to land on', async () => {
      opening.reset('landing');
      render(OpeningScreen, { props: { target: null } });
      await vi.advanceTimersByTimeAsync(0);

      expect(opening.phase).toBe('done');
    });

    it('is over after the flight is due even if it never reports its end', async () => {
      const { logo, shell } = menu();

      flights();
      opening.reset('landing');
      vi.spyOn(SVGElement.prototype, 'getBoundingClientRect').mockReturnValue(rect(340, 200, 120));
      render(OpeningScreen, { props: { target: logo } });

      await vi.advanceTimersByTimeAsync(LAND_MS + 300);

      expect(opening.phase).toBe('done');
      shell.remove();
    });
  });
});
