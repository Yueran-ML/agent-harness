import { useCallback, useEffect, useRef, useState } from 'react';
import { aboutOf, foundOf, UPDATE_EVERY_MS, UPDATE_FIRST_MS, type About, type Found } from './app/updates';

/** What the page knows of newer versions of the application, and the way to ask now. */
export interface Updates {
  /** The version that runs, and whether it looks for newer ones, once the shell has said. */
  about: About | null;
  /**
   * What the questions found: the newer version that is out, and why the last question failed,
   * where it did. Null before any answer came.
   */
  found: Found | null;
  /** Whether a question is under way. */
  asking: boolean;
  /** Ask GitHub now, and not what is known of the last hour. */
  checkNow: () => void;
}

/**
 * Newer versions of the application. A package looks for one a few seconds after the window has
 * opened and once a day after that, while the user lets it; a checkout never does. The shell asks
 * GitHub, and the page hears only what it found.
 *
 * Args:
 *   checking: Whether the application looks by itself. Asking now is done either way.
 */
export function useUpdates(checking: boolean): Updates {
  const [about, setAbout] = useState<About | null>(null);
  const [found, setFound] = useState<Found | null>(null);
  const [asking, setAsking] = useState(false);
  // The questions by number. Only the answer to the last one counts.
  const asked = useRef(0);

  useEffect(() => {
    void window.columbina?.about().then(answer => setAbout(aboutOf(answer))).catch(() => {});
  }, []);
  const checks = about?.checks === true;

  const ask = useCallback(async (now: boolean) => {
    const bridge = window.columbina;
    if (!bridge) return;
    const number = asked.current + 1;
    asked.current = number;
    setAsking(true);
    let answer: Found;
    try {
      answer = foundOf(await bridge.updates({ now }));
    } catch {
      // The shell asked nothing: there is no list, as where GitHub gave none.
      answer = foundOf(null);
    }
    if (number !== asked.current) return;
    // A version that is out stays out: a question that fails takes nothing of it away.
    setFound(before => (answer.failure !== null && before?.newer ? { newer: before.newer, failure: answer.failure } : answer));
    setAsking(false);
  }, []);

  useEffect(() => {
    if (!checks || !checking) return undefined;
    const first = setTimeout(() => void ask(false), UPDATE_FIRST_MS);
    const daily = setInterval(() => void ask(false), UPDATE_EVERY_MS);
    return () => {
      clearTimeout(first);
      clearInterval(daily);
    };
  }, [checks, checking, ask]);

  const checkNow = useCallback(() => { if (checks) void ask(true); }, [checks, ask]);
  return { about, found, asking, checkNow };
}
