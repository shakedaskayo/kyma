/**
 * Central motion vocabulary (framer-motion).
 *
 * Import variants/transitions from here so motion feels consistent across the
 * app and reduced-motion is handled in one place. CSS-level reduced-motion is
 * also enforced globally in globals.css; use `useMotionSafe()` for JS-driven
 * variants that should collapse to instant/opacity-only.
 */
import { useReducedMotion, type Transition, type Variants } from "framer-motion";

export const transitions = {
  spring: { type: "spring", stiffness: 380, damping: 32, mass: 0.8 } as Transition,
  springSoft: { type: "spring", stiffness: 220, damping: 28 } as Transition,
  easeOut: { duration: 0.2, ease: [0.16, 1, 0.3, 1] } as Transition,
  fast: { duration: 0.15, ease: "easeOut" } as Transition,
};

export const fadeIn: Variants = {
  hidden: { opacity: 0 },
  show: { opacity: 1, transition: transitions.easeOut },
  exit: { opacity: 0, transition: transitions.fast },
};

export const fadeUp: Variants = {
  hidden: { opacity: 0, y: 8 },
  show: { opacity: 1, y: 0, transition: transitions.easeOut },
  exit: { opacity: 0, y: 4, transition: transitions.fast },
};

/** Parent that staggers children appearing with `fadeUp`/`listItem`. */
export const staggerContainer: Variants = {
  hidden: {},
  show: { transition: { staggerChildren: 0.04, delayChildren: 0.02 } },
};

export const listItem: Variants = {
  hidden: { opacity: 0, y: 6 },
  show: { opacity: 1, y: 0, transition: transitions.easeOut },
};

export const modalPop: Variants = {
  hidden: { opacity: 0, scale: 0.97, y: 6 },
  show: { opacity: 1, scale: 1, y: 0, transition: transitions.spring },
  exit: { opacity: 0, scale: 0.98, y: 4, transition: transitions.fast },
};

export const panelSlideRight: Variants = {
  hidden: { opacity: 0, x: 24 },
  show: { opacity: 1, x: 0, transition: transitions.springSoft },
  exit: { opacity: 0, x: 24, transition: transitions.fast },
};

export const panelSlideLeft: Variants = {
  hidden: { opacity: 0, x: -24 },
  show: { opacity: 1, x: 0, transition: transitions.springSoft },
  exit: { opacity: 0, x: -24, transition: transitions.fast },
};

/** Hover/press feedback for interactive cards & buttons (use with `whileHover`/`whileTap`). */
export const pressable = {
  whileHover: { y: -1 },
  whileTap: { scale: 0.98 },
  transition: transitions.fast,
};

/**
 * Returns variants that respect the user's reduced-motion preference: when set,
 * transforms are stripped so only opacity changes (or nothing) remain.
 */
export function useMotionSafe<T extends Variants>(variants: T): T {
  const reduce = useReducedMotion();
  if (!reduce) return variants;
  const stripped: Record<string, unknown> = {};
  for (const key of Object.keys(variants)) {
    const v = (variants as Record<string, { opacity?: number }>)[key] ?? {};
    stripped[key] = { opacity: v.opacity ?? 1, transition: { duration: 0 } };
  }
  return stripped as T;
}

export { useReducedMotion };
