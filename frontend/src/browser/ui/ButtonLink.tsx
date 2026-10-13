import type { AnchorHTMLAttributes } from 'react';
import styles from './ButtonLink.module.css';

export interface ButtonLinkProps extends AnchorHTMLAttributes<HTMLAnchorElement> {
  variant?: 'primary' | 'secondary' | 'ghost';
  size?: 'sm' | 'md';
}

/** A link styled like a Silicon Button (which has no href), for same-site links such as "Open Browser". External links use ExternalButtonLink. */
export function ButtonLink({ variant = 'secondary', size = 'md', className, ...props }: ButtonLinkProps) {
  return <a {...props} className={[styles.link, styles[variant], styles[size], className].filter(Boolean).join(' ')} />;
}
