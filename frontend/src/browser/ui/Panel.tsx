import type { HTMLAttributes, ReactNode } from 'react';
import styles from './Panel.module.css';

export interface PanelProps extends Omit<HTMLAttributes<HTMLElement>, 'title'> {
  /** Rendered as an <h2>. */
  title?: ReactNode;
  description?: ReactNode;
  /** `subtle` uses the muted surface, for secondary or destructive sections. */
  tone?: 'default' | 'subtle';
  /** Caps the width for forms (about 36rem). */
  narrow?: boolean;
  children?: ReactNode;
}

/** A bordered surface section, for detail blocks, forms, and the sign-in panel. */
export function Panel({ title, description, tone = 'default', narrow = false, className, children, ...props }: PanelProps) {
  return <section {...props} className={[styles.panel, tone === 'subtle' ? styles.subtle : '', narrow ? styles.narrow : '', className].filter(Boolean).join(' ')}>
    {(title || description) && <div className={styles.heading}>
      {title && <h2 className={styles.title}>{title}</h2>}
      {description && <p className={styles.description}>{description}</p>}
    </div>}
    {children}
  </section>;
}
