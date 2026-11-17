import { describe, expect, it } from 'vitest';
import {
  baseName,
  isAbsolutePath,
  isRootPath,
  isSamePath,
  isUnderPath,
  isWindowsPath,
  joinPath,
  parentPath,
  rootOf,
  separatorOf,
} from './paths';

describe('path style', () => {
  it('reads a drive letter as a Windows path and everything else as POSIX', () => {
    expect(isWindowsPath('C:\\Users\\me')).toBe(true);
    expect(isWindowsPath('d:/data')).toBe(true);
    expect(isWindowsPath('C:')).toBe(true);
    expect(isWindowsPath('/home/me')).toBe(false);
    expect(isWindowsPath('C:file')).toBe(false);
    expect(separatorOf('C:\\Users')).toBe('\\');
    expect(separatorOf('/home')).toBe('/');
  });

  it('knows the roots and what is absolute', () => {
    expect(rootOf('C:\\Users\\me')).toBe('C:\\');
    expect(rootOf('/home')).toBe('/');
    expect(isRootPath('C:\\')).toBe(true);
    expect(isRootPath('C:')).toBe(true);
    expect(isRootPath('/')).toBe(true);
    expect(isRootPath('C:\\Users')).toBe(false);
    expect(isAbsolutePath('C:\\Users')).toBe(true);
    expect(isAbsolutePath('/home')).toBe(true);
    expect(isAbsolutePath('C:')).toBe(false);
    expect(isAbsolutePath('Users\\me')).toBe(false);
  });
});

describe('baseName', () => {
  it('takes the last name of either style', () => {
    expect(baseName('C:\\Users\\me\\Downloads')).toBe('Downloads');
    expect(baseName('C:\\Users\\me\\Downloads\\')).toBe('Downloads');
    expect(baseName('/dev/nvme0n1p2')).toBe('nvme0n1p2');
    expect(baseName('/home/demo/')).toBe('demo');
  });

  it('gives a root itself', () => {
    expect(baseName('C:\\')).toBe('C:\\');
    expect(baseName('/')).toBe('/');
  });
});

describe('parentPath', () => {
  it('goes up one name and keeps a root with its separator', () => {
    expect(parentPath('C:\\Users\\me')).toBe('C:\\Users');
    expect(parentPath('C:\\Users')).toBe('C:\\');
    expect(parentPath('C:\\')).toBe('C:\\');
    expect(parentPath('/home/demo')).toBe('/home');
    expect(parentPath('/home')).toBe('/');
    expect(parentPath('/')).toBe('/');
  });

  it('ignores a separator at the end', () => {
    expect(parentPath('C:\\Users\\me\\')).toBe('C:\\Users');
  });
});

describe('joinPath', () => {
  it('joins with the folder own separator', () => {
    expect(joinPath('C:\\Users\\me', 'Downloads')).toBe('C:\\Users\\me\\Downloads');
    expect(joinPath('C:\\', 'Users')).toBe('C:\\Users');
    expect(joinPath('/home/demo', 'src')).toBe('/home/demo/src');
    expect(joinPath('/', 'home')).toBe('/home');
  });
});

describe('comparing paths', () => {
  it('ignores letter case and the kind of slash on Windows only', () => {
    expect(isSamePath('c:\\users\\ME', 'C:\\Users\\me\\')).toBe(true);
    expect(isSamePath('C:/Users/me', 'C:\\Users\\me')).toBe(true);
    expect(isSamePath('/home/Me', '/home/me')).toBe(false);
  });

  it('finds a folder below another by whole names', () => {
    expect(isUnderPath('C:\\Users\\me\\x', 'c:\\users\\ME')).toBe(true);
    expect(isUnderPath('C:\\Users\\me', 'C:\\Users\\me')).toBe(true);
    expect(isUnderPath('C:\\Users\\mea', 'C:\\Users\\me')).toBe(false);
    expect(isUnderPath('C:\\Users', 'C:\\')).toBe(true);
    expect(isUnderPath('D:\\Users', 'C:\\')).toBe(false);
    expect(isUnderPath('/home/demo/x', '/home/demo')).toBe(true);
    expect(isUnderPath('/home/demox', '/home/demo')).toBe(false);
    expect(isUnderPath('/home/Demo/x', '/home/demo')).toBe(false);
    expect(isUnderPath('/anything', '/')).toBe(true);
  });
});
