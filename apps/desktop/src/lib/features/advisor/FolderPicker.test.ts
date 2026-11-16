import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError } from '../../api/client';
import type { FolderMatchDto } from '../../api/types';
import FolderPicker, { MAX_QUERY_CHARS, SEARCH_DELAY_MS } from './FolderPicker.svelte';

const FOUND: FolderMatchDto[] = [
  { node: 4, path: '/home/demo/Downloads', allocated: 96_000_000_000 },
  { node: 7, path: '/home/demo/Documents/old-downloads', allocated: 4_000_000_000 },
];

function setup(find = vi.fn().mockResolvedValue(FOUND), onenter = vi.fn()) {
  const view = render(FolderPicker, {
    props: { find, label: 'Folder to ask about', placeholder: 'Type a folder', onenter },
  });
  const input = screen.getByRole('combobox', { name: 'Folder to ask about' }) as HTMLInputElement;

  return { ...view, find, onenter, input };
}

async function type(input: HTMLInputElement, text: string): Promise<void> {
  input.value = text;
  await fireEvent.input(input);
}

/** Waits out the pause after the last key, and lets the answer come in. */
async function settle(): Promise<void> {
  await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
}

describe('FolderPicker', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('waits 150 ms after the last key before it asks, and asks once for what was typed', async () => {
    const { input, find } = setup();

    await type(input, 'Do');
    await vi.advanceTimersByTimeAsync(100);
    await type(input, 'Down');
    await vi.advanceTimersByTimeAsync(100);

    expect(find).not.toHaveBeenCalled();

    await settle();

    expect(find).toHaveBeenCalledTimes(1);
    expect(find).toHaveBeenCalledWith('Down');
  });

  it('shows each result with its path and size, and the first one is active', async () => {
    const { input } = setup();

    await type(input, 'down');
    await settle();

    const options = screen.getAllByRole('option');

    expect(options).toHaveLength(2);
    expect(options[0]?.textContent).toContain('/home/demo/Downloads');
    expect(options[0]?.textContent).toContain('96 GB');
    expect(options[0]?.getAttribute('aria-selected')).toBe('true');
    expect(input.getAttribute('aria-expanded')).toBe('true');
    expect(input.getAttribute('aria-activedescendant')).toBe(options[0]?.id);
  });

  it('moves with the arrow keys, stops at both ends, and Enter picks the active result', async () => {
    const { input } = setup();

    await type(input, 'down');
    await settle();

    await fireEvent.keyDown(input, { key: 'ArrowDown' });
    await fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(screen.getAllByRole('option')[1]?.getAttribute('aria-selected')).toBe('true');

    await fireEvent.keyDown(input, { key: 'ArrowUp' });
    await fireEvent.keyDown(input, { key: 'ArrowUp' });
    expect(screen.getAllByRole('option')[0]?.getAttribute('aria-selected')).toBe('true');

    await fireEvent.keyDown(input, { key: 'ArrowDown' });
    await fireEvent.keyDown(input, { key: 'Enter' });

    expect(input.value).toBe('/home/demo/Documents/old-downloads');
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('picks a result with the mouse', async () => {
    const { input } = setup();

    await type(input, 'down');
    await settle();
    await fireEvent.click(screen.getAllByRole('option')[0] as HTMLElement);

    expect(input.value).toBe('/home/demo/Downloads');
  });

  it('asks for the pick when Enter is pressed again with no list open', async () => {
    const { input, onenter } = setup();

    await type(input, 'down');
    await settle();
    await fireEvent.keyDown(input, { key: 'Enter' });
    expect(onenter).not.toHaveBeenCalled();

    await fireEvent.keyDown(input, { key: 'Enter' });
    expect(onenter).toHaveBeenCalledTimes(1);
  });

  it('forgets the pick when the text changes, and Enter then does nothing', async () => {
    const { input, onenter } = setup();

    await type(input, 'down');
    await settle();
    await fireEvent.keyDown(input, { key: 'Enter' });
    await type(input, '/home/demo/Downloads/x');
    await fireEvent.keyDown(input, { key: 'Enter' });

    expect(onenter).not.toHaveBeenCalled();
  });

  it('closes on Escape and when the focus leaves', async () => {
    const { input } = setup();

    await type(input, 'down');
    await settle();
    await fireEvent.keyDown(input, { key: 'Escape' });
    expect(screen.queryByRole('listbox')).toBeNull();

    await type(input, 'downl');
    await settle();
    expect(screen.getByRole('listbox')).toBeTruthy();

    await fireEvent.focusOut(input, { relatedTarget: document.body });
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('takes at most 255 characters', () => {
    const { input } = setup();

    expect(input.maxLength).toBe(MAX_QUERY_CHARS);
    expect(MAX_QUERY_CHARS).toBe(255);
  });

  it('drops the answer of an earlier text', async () => {
    let first: (found: FolderMatchDto[]) => void = () => {};
    const find = vi
      .fn()
      .mockImplementationOnce(() => new Promise<FolderMatchDto[]>((resolve) => (first = resolve)))
      .mockResolvedValueOnce([{ node: 9, path: '/second', allocated: 1 }]);
    const { input } = setup(find);

    await type(input, 'a');
    await settle();
    await type(input, 'ab');
    await settle();
    first(FOUND);
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.getAllByRole('option').map((option) => option.textContent)).toEqual([
      expect.stringContaining('/second'),
    ]);
  });

  it('says so when no folder matches, and shows the message of a failed search as text', async () => {
    const { input, find } = setup(vi.fn().mockResolvedValue([]));

    await type(input, 'zzz');
    await settle();
    expect(screen.getByText('No folder matches.')).toBeTruthy();

    find.mockRejectedValueOnce(new ApiRequestError('invalid', 'The scan is gone.'));
    await type(input, 'zzzz');
    await settle();

    expect(screen.getByRole('alert').textContent).toBe('The scan is gone.');
  });

  it('shows nothing for an empty text, and makes control characters in a path visible', async () => {
    const find = vi.fn().mockResolvedValue([{ node: 1, path: '/tmp/a‮b', allocated: 5 }]);
    const { input } = setup(find);

    await type(input, '   ');
    await settle();
    expect(find).not.toHaveBeenCalled();

    await type(input, 'a');
    await settle();

    expect(screen.getByRole('option').textContent).toContain('/tmp/a\\u{202E}b');
  });

  it('is off when disabled', () => {
    render(FolderPicker, {
      props: { find: vi.fn(), label: 'Folder', placeholder: 'x', disabled: true },
    });

    expect((screen.getByRole('combobox') as HTMLInputElement).disabled).toBe(true);
  });
});
