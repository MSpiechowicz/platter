import { describe, expect, it } from 'vitest';
import type { VolumeDto } from '../../api/types';
import { countDisks, groupVolumes } from './volumes';

const volume = (id: string, mountPoint: string, device = '/dev/nvme0n1p2', available = 400): VolumeDto => ({
  id,
  mountPoint,
  label: '',
  device,
  fsType: 'btrfs',
  total: 1000,
  available,
  removable: false,
});

describe('groupVolumes', () => {
  it('shows one row for the mounts of one filesystem, with the shortest mount point and the rest listed', () => {
    const groups = groupVolumes([
      volume('home', '/home'),
      volume('root', '/'),
      volume('srv', '/srv'),
      volume('log', '/var/log'),
    ]);

    expect(groups).toHaveLength(1);
    expect(groups[0]?.volume.id).toBe('root');
    expect(groups[0]?.alsoMountedAt).toEqual(['/srv', '/home', '/var/log']);
  });

  it('keeps volumes apart when the device or the size differs', () => {
    const groups = groupVolumes([
      volume('a', '/a'),
      volume('b', '/b', '/dev/sda1'),
      { ...volume('c', '/c'), total: 2000 },
      { ...volume('d', '/d'), fsType: 'ext4' },
    ]);

    expect(groups.map((group) => group.volume.id)).toEqual(['a', 'b', 'c', 'd']);
    expect(groups.every((group) => group.alsoMountedAt.length === 0)).toBe(true);
  });

  it('keeps one row for a busy disk whose free space was read at slightly different moments', () => {
    const groups = groupVolumes([
      volume('home', '/home', '/dev/nvme0n1p2', 398),
      volume('root', '/', '/dev/nvme0n1p2', 400),
      volume('srv', '/srv', '/dev/nvme0n1p2', 399),
    ]);

    expect(groups).toHaveLength(1);
    expect(groups[0]?.volume.id).toBe('root');
    expect(groups[0]?.volume.available).toBe(400);
    expect(groups[0]?.alsoMountedAt).toEqual(['/srv', '/home']);
  });

  it.each(['', 'none', 'btrfs', 'tmpfs', 'overlay'])(
    'never merges volumes whose device is %j, which is no device',
    (device) => {
      const fake = (id: string, mountPoint: string): VolumeDto => ({
        ...volume(id, mountPoint, device),
        fsType: device === '' ? 'btrfs' : device,
      });
      const groups = groupVolumes([fake('a', '/a'), fake('b', '/b'), fake('c', '/c')]);

      expect(groups.map((group) => group.volume.id)).toEqual(['a', 'b', 'c']);
      expect(groups.every((group) => group.alsoMountedAt.length === 0)).toBe(true);
    },
  );

  it('keeps the order of the first appearance and lists a repeated mount point once', () => {
    const groups = groupVolumes([
      volume('usb', '/media/usb', '/dev/sdb1'),
      volume('root', '/'),
      volume('home', '/home'),
      volume('home-again', '/home'),
    ]);

    expect(groups.map((group) => group.volume.id)).toEqual(['usb', 'root']);
    expect(groups[1]?.alsoMountedAt).toEqual(['/home']);
  });

  it('is empty for no volumes', () => {
    expect(groupVolumes([])).toEqual([]);
  });
});

describe('countDisks', () => {
  it('counts btrfs subvolumes and the partitions of one drive as one disk', () => {
    const boot = { ...volume('boot', '/boot', '/dev/nvme0n1p1'), fsType: 'vfat', total: 2000 };

    expect(
      countDisks([
        volume('root', '/'),
        volume('home', '/home'),
        volume('log', '/var/log'),
        volume('cache', '/var/cache'),
        boot,
      ]),
    ).toBe(1);
  });

  it('counts separate drives, macOS slices and devices it can not place', () => {
    expect(
      countDisks([
        volume('root', '/', '/dev/sda2'),
        { ...volume('data', '/data', '/dev/sdb1'), total: 3000 },
        { ...volume('mac', '/Volumes/X', '/dev/disk3s1s1'), total: 4000 },
        { ...volume('mac2', '/Volumes/Y', '/dev/disk3s5'), total: 5000 },
        { ...volume('crypt', '/srv', '/dev/mapper/luks-1'), total: 6000 },
        { ...volume('tmp', '/tmp', 'tmpfs'), fsType: 'tmpfs', total: 7000 },
      ]),
    ).toBe(5);
  });
});
