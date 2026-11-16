import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import ExplorerLegend from './ExplorerLegend.svelte';

describe('ExplorerLegend', () => {
  it('says "Bigger tile = more space" while every tile is drawn at its size', () => {
    render(ExplorerLegend, { props: { view: 'flat' } });

    expect(screen.getByText('Bigger tile = more space')).toBeTruthy();
  });

  it('says that small folders are drawn larger as soon as one tile is', () => {
    render(ExplorerLegend, { props: { view: 'flat', boosted: true } });

    expect(
      screen.getByText('Bigger tile = more space · small folders are drawn larger so they can be read'),
    ).toBeTruthy();
  });

  it("says the same under the 3D view's own text", () => {
    render(ExplorerLegend, { props: { view: '3d', boosted: true } });

    expect(
      screen.getByText(
        'Column = folder · floor = sub-folder · volume = space · small folders are drawn larger so they can be read',
      ),
    ).toBeTruthy();
  });
});

describe('ExplorerLegend keys', () => {
  it('has "Safe to clean", "Marked for cleanup" and "Selected", in both views', () => {
    for (const view of ['flat', '3d'] as const) {
      const { unmount } = render(ExplorerLegend, { props: { view } });

      expect(screen.getByText('Safe to clean')).toBeTruthy();
      expect(screen.getByText('Marked for cleanup')).toBeTruthy();
      expect(screen.getByText('Selected')).toBeTruthy();
      unmount();
    }
  });

  it('says so when the open folder is inside a marked one', () => {
    render(ExplorerLegend, { props: { view: 'flat', insideMarked: true } });

    expect(screen.getByText('This folder is inside one that is marked for cleanup')).toBeTruthy();
  });
});
