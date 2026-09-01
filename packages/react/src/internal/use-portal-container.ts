/**
 * Returns the portal container from the nearest PensieveContext.
 * Radix portal components (tooltips, dropdowns, dialogs) must render inside
 * this element so they inherit the .pensieve-root CSS variables.
 */
import { usePensieveContext } from "../provider/context";

export function usePortalContainer(): HTMLElement | null {
  return usePensieveContext().portalContainer;
}
