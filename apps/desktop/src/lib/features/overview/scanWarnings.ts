import type { ScanWarning } from '../../api/types';

/** What each scan warning means for the numbers on the page, in one line. */
const WARNING_TEXT: Record<ScanWarning, string> = {
  mountTableUnavailable: "Mounted disks could not be read, so only this disk's own files were counted",
  excludedUnresolved:
    'Some folders you leave out of scans could not be found, so they were scanned this time',
};

/** The lines to show under the status of a finished scan, one per warning, without repeats. */
export function warningLines(warnings: readonly ScanWarning[]): string[] {
  return [...new Set(warnings)].map((warning) => WARNING_TEXT[warning]);
}
