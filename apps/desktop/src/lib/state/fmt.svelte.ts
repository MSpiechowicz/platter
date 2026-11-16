import { formatSize } from '../format';
import { settings } from './settings.svelte';

/** Formatters that follow the size-unit setting. Reading `units` inside a template makes it re-render on change. */
export const fmt = {
  size: (bytes: number): string => formatSize(bytes, settings.units),
};
