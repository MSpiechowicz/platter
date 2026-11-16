import type { MockTimers } from './mock-advisor';

/** Timers that only move when the test says so. */
export class FakeTimers implements MockTimers {
  private now = 0;
  private next = 1;
  private readonly pending = new Map<number, { at: number; callback: () => void }>();

  set(callback: () => void, ms: number): unknown {
    const id = this.next++;
    this.pending.set(id, { at: this.now + ms, callback });

    return id;
  }

  clear(handle: unknown): void {
    this.pending.delete(handle as number);
  }

  get count(): number {
    return this.pending.size;
  }

  /** Runs everything that falls due within `ms`, in time order, including timers set meanwhile. */
  advance(ms: number): void {
    const end = this.now + ms;

    for (;;) {
      const due = [...this.pending.entries()]
        .filter(([, timer]) => timer.at <= end)
        .sort((a, b) => a[1].at - b[1].at || a[0] - b[0])[0];

      if (!due) {
        break;
      }

      this.pending.delete(due[0]);
      this.now = due[1].at;
      due[1].callback();
    }

    this.now = end;
  }
}
