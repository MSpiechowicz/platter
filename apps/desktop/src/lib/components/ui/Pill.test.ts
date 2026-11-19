import { render } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Pill from './Pill.svelte';

const text = createRawSnippet(() => ({ render: () => '<span>Scanned just now · 3,724,223 files</span>' }));

describe('Pill', () => {
  it('may be cut only when asked to, and then carries the whole text as its tooltip', () => {
    const plain = render(Pill, { props: { children: text } });
    const pill = plain.container.querySelector('.pill') as HTMLElement;

    expect(pill.classList.contains('truncate')).toBe(false);
    expect(pill.getAttribute('title')).toBeNull();
    plain.unmount();

    const cut = render(Pill, { props: { children: text, truncate: true, title: 'the whole status' } });
    const truncating = cut.container.querySelector('.pill') as HTMLElement;

    expect(truncating.classList.contains('truncate')).toBe(true);
    expect(truncating.getAttribute('title')).toBe('the whole status');
  });
});
