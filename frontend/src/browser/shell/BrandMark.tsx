/** The Team of Silicons mark (public/mark.svg), drawn in the current text colour so it follows the theme. */
export function BrandMark({ size = 28, className }: { size?: number; className?: string }) {
  return <svg className={className} width={size} height={size} viewBox="0 0 355 355" fill="currentColor" aria-hidden="true" focusable="false">
    <rect y="131.865" width="91.286" height="91.287" />
    <rect x="131.86" width="91.286" height="91.287" />
    <rect x="263.718" y="131.865" width="91.286" height="91.287" />
    <rect x="131.86" y="263.717" width="91.286" height="91.287" />
    <rect x="40.569" y="40.568" width="91.286" height="91.287" />
    <rect x="223.139" y="40.568" width="91.286" height="91.287" />
    <rect x="40.569" y="223.141" width="91.286" height="91.287" />
    <rect x="223.139" y="223.141" width="91.286" height="91.287" />
    <rect width="139.285" height="139.285" transform="matrix(0.7071 -0.7071 0.7071 0.7071 78.569 177.502)" />
  </svg>;
}

/** Mark plus the "Browser" wordmark. As a link it always does a full page load of `/`. */
export function Brand({ link = true, className }: { link?: boolean; className?: string }) {
  const content = <><BrandMark /><span>Browser</span></>;
  return link ? <a className={className} href="/" aria-label="Browser home">{content}</a> : <div className={className}>{content}</div>;
}
