/** A length of time in the parts that a catalogue puts into words. */
export interface DurationParts {
  hours: number;
  minutes: number;
  seconds: number;
}

/**
 * Split a number of seconds into hours, minutes and seconds.
 *
 * Below ten seconds a tenth of a second is kept, as in half a second. From
 * there on whole seconds are close enough. What is no length of time comes
 * to nothing.
 */
export function splitDuration(seconds: number): DurationParts {
  if (!Number.isFinite(seconds) || seconds <= 0) return { hours: 0, minutes: 0, seconds: 0 };
  const tenths = Math.round(seconds * 10) / 10;
  // Rounded before it is split, so that 59.6 seconds come to a minute and not to sixty seconds.
  const rounded = tenths < 10 ? tenths : Math.round(seconds);
  const hours = Math.floor(rounded / 3600);
  const minutes = Math.floor((rounded - hours * 3600) / 60);
  return { hours, minutes, seconds: rounded - hours * 3600 - minutes * 60 };
}
