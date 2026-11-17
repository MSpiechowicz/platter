import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { CleanupPlanDto, PlanGroupDto } from '../../api/types';
import { fmt } from '../../state/fmt.svelte';
import ReviewDialog from './ReviewDialog.svelte';

const GB = 1_000_000_000;

const marked = (paths: string[], items = paths.length): PlanGroupDto => ({
  ruleId: 'marked',
  title: 'Marked in the Explorer',
  verdict: 'review',
  action: 'trash',
  items,
  allocated: 5 * GB,
  needsElevation: false,
  samplePaths: paths,
});

const plan = (groups: PlanGroupDto[], extra: Partial<CleanupPlanDto> = {}): CleanupPlanDto => ({
  planId: 1,
  scanId: 1,
  options: { keepPackageVersions: 2, askForAdmin: true },
  groups,
  refused: [],
  refusedMore: 0,
  notFound: [],
  allocated: groups.reduce((sum, group) => sum + group.allocated, 0),
  needsConfirmation: true,
  ...extra,
});

const show = (shown: CleanupPlanDto, onconfirm = vi.fn(), oncancel = vi.fn()) =>
  render(ReviewDialog, { props: { plan: shown, suggestions: [], onconfirm, oncancel } });

describe('ReviewDialog for what the user marked', () => {
  it('counts every marked item, lists each path under the one group, and says the user chose them', async () => {
    show(plan([marked(['/home/demo/Downloads', '/home/demo/.cache'])]));

    const dialog = await screen.findByRole('dialog');

    expect(within(dialog).getByRole('heading', { name: 'Clean 2 items that need review?' })).toBeTruthy();
    expect(dialog.textContent).toContain('Marked in the Explorer');
    expect(dialog.textContent).toContain('/home/demo/Downloads');
    expect(dialog.textContent).toContain('/home/demo/.cache');
    expect(dialog.textContent).toContain('You marked these yourself');
    expect(dialog.textContent).toContain('Moved to the trash. This can be undone.');
    expect(dialog.textContent).toContain(`Total ${fmt.size(5 * GB)}`);
  });

  it('says "1 item that needs review" for one marked item', async () => {
    show(plan([marked(['/home/demo/Downloads'])]));

    expect(await screen.findByRole('heading', { name: 'Clean 1 item that needs review?' })).toBeTruthy();
  });

  it('says how many marked items are not listed, and shows hidden characters in a path', async () => {
    show(plan([marked(['/home/a‮b'], 30)]));

    const dialog = await screen.findByRole('dialog');

    expect(dialog.textContent).toContain('/home/a\\u{202E}b');
    expect(dialog.textContent).toContain('… and 29 more');
    expect(within(dialog).getByRole('heading', { name: 'Clean 30 items that need review?' })).toBeTruthy();
  });

  it('shows the refusals as plain text', async () => {
    show(
      plan([marked(['/home/demo/Downloads'])], {
        refused: [{ ruleId: 'marked', path: '/home/demo/Videos', reason: 'kept on purpose' }],
      }),
    );

    expect((await screen.findByRole('dialog')).textContent).toContain(
      'Not cleaned: /home/demo/Videos (kept on purpose)',
    );
  });

  it('confirms and cancels through the buttons', async () => {
    const onconfirm = vi.fn();
    const oncancel = vi.fn();

    show(plan([marked(['/home/demo/Downloads'])]), onconfirm, oncancel);

    const dialog = await screen.findByRole('dialog');

    await fireEvent.click(within(dialog).getByRole('button', { name: `Clean ${fmt.size(5 * GB)}` }));
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));

    expect(onconfirm).toHaveBeenCalledOnce();
    expect(oncancel).toHaveBeenCalledOnce();
  });
});
