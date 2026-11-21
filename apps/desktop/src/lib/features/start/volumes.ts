import type { VolumeDto } from '../../api/types';

/** One physical filesystem: the volume to show, and the other places it is mounted. */
export interface VolumeGroup {
  volume: VolumeDto;
  /** The other mount points, shortest first. */
  alsoMountedAt: string[];
}

/**
 * Whether `device` names one real device: a path (`/dev/sda1`, `//host/share`), a network source (`host:/export`)
 * or a Windows volume. An empty string, `none`, `tmpfs` or the filesystem type itself name nothing, so two volumes
 * that both say so are not the same disk.
 */
function isDeviceIdentifier(volume: VolumeDto): boolean {
  const device = volume.device.trim();

  if (device === '' || device.toLowerCase() === volume.fsType.toLowerCase()) {
    return false;
  }

  return /^[/\\]|^[A-Za-z]:|:\//.test(device);
}

/**
 * The free space is left out of the key: it is read per mount at slightly different moments, so a disk that is
 * being written to would split into several rows.
 */
const groupKey = (volume: VolumeDto): string => `${volume.device}\0${volume.fsType}\0${volume.total}`;

/**
 * Volumes that are the same filesystem mounted several times (btrfs subvolumes: /, /home, /srv, ...) become one
 * group. They share the device, the type and the size; the one with the shortest mount point is shown, with its
 * own free space. A volume without a real device identifier is never grouped.
 * Groups keep the order in which their first volume appeared.
 */
export function groupVolumes(volumes: readonly VolumeDto[]): VolumeGroup[] {
  const members = new Map<string, VolumeDto[]>();

  volumes.forEach((volume, index) => {
    const key = isDeviceIdentifier(volume) ? groupKey(volume) : `\0alone\0${index}`;
    const group = members.get(key);

    if (group) {
      group.push(volume);
    } else {
      members.set(key, [volume]);
    }
  });

  return [...members.values()].map((group) => {
    const ordered = [...group].sort(
      (a, b) => a.mountPoint.length - b.mountPoint.length || a.mountPoint.localeCompare(b.mountPoint),
    );
    const [volume] = ordered as [VolumeDto, ...VolumeDto[]];
    const others = ordered.map((member) => member.mountPoint).filter((path) => path !== volume.mountPoint);

    return { volume, alsoMountedAt: [...new Set(others)] };
  });
}

/**
 * The disk a device node is a partition of: `/dev/nvme0n1p2` and `/dev/nvme0n1p1` are both on `/dev/nvme0n1`,
 * `/dev/sda1` is on `/dev/sda`, `/dev/disk3s1s1` (macOS) on `/dev/disk3`. Anything else (a drive letter, a network
 * share, a mapped device) is taken as its own disk.
 */
function diskOf(device: string): string {
  const nvme = /^(\/dev\/(?:nvme\d+n\d+|mmcblk\d+|loop\d+))p\d+$/.exec(device);

  if (nvme) {
    return nvme[1] as string;
  }

  const lettered = /^(\/dev\/(?:[shv]d[a-z]+|xvd[a-z]+))\d+$/.exec(device);

  if (lettered) {
    return lettered[1] as string;
  }

  const mac = /^(\/dev\/disk\d+)(?:s\d+)+$/.exec(device);

  return mac ? (mac[1] as string) : device;
}

/**
 * How many disks the volumes are on, as a person counts them: btrfs subvolumes are one filesystem, and the
 * partitions of one drive (a root and its `/boot`) are one disk.
 */
export function countDisks(volumes: readonly VolumeDto[]): number {
  const disks = new Set(
    groupVolumes(volumes).map(({ volume }, index) =>
      isDeviceIdentifier(volume) ? diskOf(volume.device.trim()) : `\0alone\0${index}`,
    ),
  );

  return disks.size;
}
