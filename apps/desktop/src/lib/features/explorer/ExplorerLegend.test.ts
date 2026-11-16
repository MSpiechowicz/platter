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
