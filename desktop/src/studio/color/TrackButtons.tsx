import { useWords } from '../../app/locale';
import type { TrackDirection } from '../engine';
import './tracker.css';

/** A button of a tracker: a direction to track in, or Stop. */
export type TrackButton = TrackDirection | 'stop';

type Word = 'trackOneReverse' | 'trackReverse' | 'stop' | 'trackForward' | 'trackForwardBack' | 'trackOneForward';
const FACES: Record<TrackButton, { glyph: string; word: Word }> = {
  one_reverse: { glyph: '◀|', word: 'trackOneReverse' },
  reverse: { glyph: '◀', word: 'trackReverse' },
  stop: { glyph: '■', word: 'stop' },
  forward: { glyph: '▶', word: 'trackForward' },
  forward_and_back: { glyph: '◀▶', word: 'trackForwardBack' },
  one_forward: { glyph: '|▶', word: 'trackOneForward' },
};

/** The analyze buttons in the order of the Tracker palette of the Color page (p. 3236). */
export const PALETTE_BUTTONS: readonly TrackButton[] = ['one_reverse', 'reverse', 'stop', 'forward', 'forward_and_back', 'one_forward'];

/**
 * The buttons that track and stop a track, each named as DaVinci names it: the Tracker palette's of the Color page and the
 * Tracker's of the Fusion page, each in the order of its own. While a track goes on Stop alone is on; while none does, Stop
 * is off and the others are on, unless `disabled` turns them off too.
 */
export function TrackButtons({ running, buttons = PALETTE_BUTTONS, disabled = false, onStart, onStop }: {
  running: boolean;
  buttons?: readonly TrackButton[];
  /** Whether nothing may be tracked now, as of a clip on a locked track: Stop still stops a track that goes on. */
  disabled?: boolean;
  onStart: (direction: TrackDirection) => void;
  onStop: () => void;
}) {
  const said = useWords().label.studio.colorPage.trackerPalette;
  return <div className="tracker-analyze" role="group" aria-label={said.analyze}>
    {buttons.map((direction) => {
      const { glyph, word } = FACES[direction];
      return <button key={direction} type="button" className="icon-btn" data-test={`tracker-${direction}`} aria-label={said[word]} title={said[word]}
        disabled={direction === 'stop' ? !running : running || disabled}
        onClick={() => (direction === 'stop' ? onStop() : onStart(direction))}>{glyph}</button>;
    })}
  </div>;
}
