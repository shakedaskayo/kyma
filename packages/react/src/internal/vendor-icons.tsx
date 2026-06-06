// Real vendor marks for brands that `@icons-pack/react-simple-icons` no longer
// ships (Simple Icons removed Slack and Amazon S3 for trademark reasons). These
// are the official logos, inlined so the catalog shows recognisable brand marks
// rather than generic glyphs. `monochrome` collapses them to `currentColor`.
// Copied from web/src/features/connectors/vendor-icons.tsx (do not import from web).

export interface VendorIconProps {
  size?: number;
  monochrome?: boolean;
  className?: string;
}

/** Official 4-colour Slack mark. */
export function SlackIcon({ size = 18, monochrome = false, className }: VendorIconProps) {
  const c = (color: string) => (monochrome ? "currentColor" : color);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 122.8 122.8"
      className={className}
      role="img"
      aria-label="Slack"
    >
      <path
        fill={c("#36C5F0")}
        d="M25.8 77.6c0 7.1-5.8 12.9-12.9 12.9S0 84.7 0 77.6s5.8-12.9 12.9-12.9h12.9v12.9zm6.5 0c0-7.1 5.8-12.9 12.9-12.9s12.9 5.8 12.9 12.9v32.3c0 7.1-5.8 12.9-12.9 12.9s-12.9-5.8-12.9-12.9V77.6z"
      />
      <path
        fill={c("#2EB67D")}
        d="M45.2 25.8c-7.1 0-12.9-5.8-12.9-12.9S38.1 0 45.2 0s12.9 5.8 12.9 12.9v12.9H45.2zm0 6.5c7.1 0 12.9 5.8 12.9 12.9s-5.8 12.9-12.9 12.9H12.9C5.8 58.1 0 52.3 0 45.2s5.8-12.9 12.9-12.9h32.3z"
      />
      <path
        fill={c("#ECB22E")}
        d="M97 45.2c0-7.1 5.8-12.9 12.9-12.9s12.9 5.8 12.9 12.9-5.8 12.9-12.9 12.9H97V45.2zm-6.5 0c0 7.1-5.8 12.9-12.9 12.9s-12.9-5.8-12.9-12.9V12.9C64.7 5.8 70.5 0 77.6 0s12.9 5.8 12.9 12.9v32.3z"
      />
      <path
        fill={c("#E01E5A")}
        d="M77.6 97c7.1 0 12.9 5.8 12.9 12.9s-5.8 12.9-12.9 12.9-12.9-5.8-12.9-12.9V97h12.9zm0-6.5c-7.1 0-12.9-5.8-12.9-12.9s5.8-12.9 12.9-12.9h32.3c7.1 0 12.9 5.8 12.9 12.9s-5.8 12.9-12.9 12.9H77.6z"
      />
    </svg>
  );
}

/** Amazon S3 — the green storage-bucket mark. */
export function AmazonS3Icon({ size = 18, monochrome = false, className }: VendorIconProps) {
  const dark = monochrome ? "currentColor" : "#1B660F";
  const light = monochrome ? "currentColor" : "#6CAE3E";
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      className={className}
      role="img"
      aria-label="Amazon S3"
    >
      {/* bucket body */}
      <path
        fill={dark}
        d="M5 6.6h14l-1.32 12.04a1.6 1.6 0 0 1-1.59 1.43H7.91a1.6 1.6 0 0 1-1.59-1.43L5 6.6z"
      />
      {/* rim */}
      <ellipse cx="12" cy="6.4" rx="7" ry="2.2" fill={light} />
      {/* highlight band */}
      <path
        fill={light}
        opacity={monochrome ? 1 : 0.85}
        d="M6.06 9.9c1.5.9 3.66 1.46 5.94 1.46s4.44-.56 5.94-1.46l-.34 3.1c-1.5.78-3.46 1.25-5.6 1.25s-4.1-.47-5.6-1.25L6.06 9.9z"
      />
    </svg>
  );
}
