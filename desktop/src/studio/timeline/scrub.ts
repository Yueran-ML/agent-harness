import { SCRUB_EVERY_MS } from '../Transport';

/** Same leading/trailing throttle as the source scrubber; a release cancels queued approximations. */
export class Scrub {
  private at = -Infinity;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private waiting: number | null = null;
  constructor(private readonly seek: (frame: number, scrubbing: boolean) => void) {}
  drag(frame: number): void {
    this.waiting = frame;
    const wait = SCRUB_EVERY_MS - (Date.now() - this.at);
    if (wait <= 0 && this.timer === null) this.send();
    else if (this.timer === null) this.timer = setTimeout(() => { this.timer = null; this.send(); }, Math.max(0, wait));
  }
  private send(): void {
    const frame = this.waiting;
    this.waiting = null;
    if (frame !== null) { this.at = Date.now(); this.seek(frame, true); }
  }
  finish(frame: number): void { this.cancel(); this.seek(frame, false); }
  cancel(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null; this.waiting = null; this.at = -Infinity;
  }
}
