import { createContext } from "react";

/** Where the app is, plus an optional place within the page to jump to. */
export interface NavTarget {
  page: string;
  /** A sub-section of the page (Control's sections). */
  section?: string;
  /** Text to pre-fill in the page's own search or filter. */
  query?: string;
  /** A card on the page to scroll to, by its title. */
  card?: string;
  /** Changes on every navigation, so repeating the same jump still triggers. */
  nonce: number;
}

export const NavContext = createContext<{
  target: NavTarget;
  go: (to: Omit<NavTarget, "nonce">) => void;
}>({ target: { page: "live", nonce: 0 }, go: () => {} });
