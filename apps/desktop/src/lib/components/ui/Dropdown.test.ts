import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import Dropdown from './Dropdown.svelte';

const OPTIONS = [
  { value: 'day', label: 'Once a day' },
  { value: 'week', label: 'Once a week' },
  { value: 'never', label: 'Never' },
];

function setup(onchange = vi.fn()) {
  render(Dropdown, { props: { options: OPTIONS, value: 'day', label: 'Check for updates', onchange } });

  return { onchange, trigger: screen.getByRole('button', { name: /Check for updates/ }) };
}

describe('Dropdown', () => {
  it('shows the current choice and a closed menu', () => {
    const { trigger } = setup();

    expect(trigger.textContent).toContain('Once a day');
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('opens on click and marks the selected option', async () => {
    const { trigger } = setup();

    await fireEvent.click(trigger);

    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    const options = screen.getAllByRole('option');
    expect(options.map((option) => option.getAttribute('aria-selected'))).toEqual(['true', 'false', 'false']);
  });

  it('puts the focus on the selected option when it opens', async () => {
    const { trigger } = setup();

    await fireEvent.click(trigger);
    await Promise.resolve();

    expect(document.activeElement?.textContent).toContain('Once a day');
  });

  it('selects with a click, reports the value and closes', async () => {
    const { onchange, trigger } = setup();

    await fireEvent.click(trigger);
    await fireEvent.click(screen.getByRole('option', { name: /Once a week/ }));

    expect(onchange).toHaveBeenCalledWith('week');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(trigger.textContent).toContain('Once a week');
  });

  it('moves with the arrow keys and closes on Escape, returning the focus to the button', async () => {
    const { onchange, trigger } = setup();

    await fireEvent.click(trigger);
    await Promise.resolve();

    const listbox = screen.getByRole('listbox');

    await fireEvent.keyDown(listbox, { key: 'ArrowDown' });
    expect(document.activeElement?.textContent).toContain('Once a week');

    await fireEvent.keyDown(listbox, { key: 'End' });
    expect(document.activeElement?.textContent).toContain('Never');

    await fireEvent.keyDown(listbox, { key: 'Escape' });
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    expect(onchange).not.toHaveBeenCalled();
  });

  it('closes when the user clicks elsewhere', async () => {
    const { trigger } = setup();

    await fireEvent.click(trigger);
    await fireEvent.click(document.body);

    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('is as wide as its longest option: every label is rendered once in a hidden sizer', async () => {
    const { trigger } = setup();
    const sizer = trigger.parentElement?.querySelector('.sizer');

    expect(sizer?.getAttribute('aria-hidden')).toBe('true');
    expect(sizer?.textContent).toContain('Once a week');
    expect(sizer?.textContent).toContain('Never');
  });
});
