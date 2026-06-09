/**
 * Returns the portal container from the nearest KymaContext.
 * Radix portal components (tooltips, dropdowns, dialogs) must render inside
 * this element so they inherit the .kyma-root CSS variables.
 */
import { useKymaContext } from "../provider/context";

export function usePortalContainer(): HTMLElement | null {
  return useKymaContext().portalContainer;
}
