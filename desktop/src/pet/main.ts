import './pet.css';
import { measureIn, startPet } from './window';

// The pet moves at every frame of the screen, and not at all where the system asks for less motion.
const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
if (window.columbinaPet) {
  void startPet(document, window.columbinaPet, measureIn(document), () => performance.now(), still).then((pet) => {
    const frame = (now: number) => { pet.draw(now); requestAnimationFrame(frame); };
    requestAnimationFrame(frame);
  });
}
