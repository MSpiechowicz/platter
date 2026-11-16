import { baseName, formatSize, type SizeUnits } from '../../format';
import type { VolumeDto } from '../../api/types';
import { findVolume } from '../../state/scan.svelte';

export interface Fact {
  label: string;
  value: string;
}

/** What is known about the place that is about to be (or is being) scanned, from the volume list. */
export function describeTarget(volumes: VolumeDto[], root: string | null, units: SizeUnits): Fact[] {
  if (root === null) {
    return [];
  }

  const found = findVolume(volumes, root);

  if (!found) {
    return [{ label: 'Folder', value: root }];
  }

  const { volume, exact } = found;
  const free = { label: 'Free', value: formatSize(volume.available, units) };
  const used = { label: 'Used', value: formatSize(volume.total - volume.available, units) };
  const filesystem = { label: 'Filesystem', value: volume.fsType };

  if (exact) {
    return [used, free, filesystem, { label: 'Device', value: baseName(volume.device) }];
  }

  return [
    { label: 'Folder', value: root },
    { label: 'On disk', value: baseName(volume.device) },
    filesystem,
    free,
  ];
}
