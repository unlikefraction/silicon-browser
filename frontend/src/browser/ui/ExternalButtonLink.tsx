import type { ReactNode } from 'react';
import { safeHttps } from '@/lib/api';
import { ButtonLink, type ButtonLinkProps } from './ButtonLink';
import styles from './ButtonLink.module.css';

export interface ExternalButtonLinkProps extends Pick<ButtonLinkProps, 'variant' | 'size' | 'className'> {
  /** Rendered only when it is a credential-free https URL (safeHttps); otherwise nothing renders. */
  href?: string;
  children: ReactNode;
}

/** An external link styled like a Silicon Button. Opens a new tab without opener or referrer. */
export function ExternalButtonLink({ href, children, ...props }: ExternalButtonLinkProps) {
  const url = safeHttps(href);
  if (!url) return null;
  return <ButtonLink {...props} href={url} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer">
    {children}<span aria-hidden="true">↗</span><span className={styles.hint}> (opens in a new tab)</span>
  </ButtonLink>;
}
