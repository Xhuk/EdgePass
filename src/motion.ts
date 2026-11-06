/**
 * Motion house style.
 *
 * Apple describes springs with two designer-facing numbers — **damping ratio**
 * (overshoot) and **response** (how fast it settles) — instead of
 * mass/stiffness/damping (Designing Fluid Interfaces, WWDC 2018). Motion's
 * `bounce` + `duration` maps onto exactly those two, so what follows are
 * transcriptions rather than hand-tuned magic numbers:
 *
 *   move / reposition      damping 1.0   response 0.4
 *   drawer / sheet         damping 0.8   response 0.3
 *
 * House rules:
 * - Anything that merely *appears* is critically damped (bounce 0). Overshoot
 *   on a menu that just faded in reads as sloppy.
 * - Bounce is reserved for things the user actually threw.
 * - Press feedback has to feel instant on the way down, so it is short.
 *
 * The window morph itself is not here: moving a native window can't be
 * composited, so it is integrated by hand in `dock.ts`.
 */

export const spring = {
  /** Window geometry and layout morphs: never overshoot a window's bounds. */
  move: { type: "spring", bounce: 0, duration: 0.4 },
  /** Panel opening: a touch of life, still no wobble. */
  sheet: { type: "spring", bounce: 0.14, duration: 0.34 },
  /** Content fading in: crisp, no bounce. */
  content: { type: "spring", bounce: 0, duration: 0.26 },
  /** Material arriving (toast, alert). */
  material: { type: "spring", bounce: 0, duration: 0.42 },
  /** Press: must read as instant. */
  press: { type: "spring", bounce: 0, duration: 0.14 },
} as const;

/** Every control answers a press the same way. */
export const tap = { scale: 0.965, transition: spring.press } as const;
