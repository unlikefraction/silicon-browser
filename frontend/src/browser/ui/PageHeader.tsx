import { createContext, useContext, type ReactNode } from 'react';
import styles from './PageHeader.module.css';

/**
 * What the shell shows right after the page header (the "Add an account" panel), so the page's <h1> stays its first heading.
 * WorkspaceView provides it; screens never set it.
 */
export const AfterPageHeader = createContext<ReactNode>(null);

export interface PageHeaderProps {
  /** Small label above the title, e.g. "Workspace" (shown in capitals by CSS, so screen readers read a word, not letters). */
  eyebrow?: string;
  title: ReactNode;
  subtitle?: ReactNode;
  /** Buttons or a status badge, aligned to the end of the header (they wrap below on narrow screens). */
  actions?: ReactNode;
}

/** The heading block every workspace view starts with. Renders the page's only <h1>, then whatever the shell placed after it. */
export function PageHeader({ eyebrow, title, subtitle, actions }: PageHeaderProps) {
  const after = useContext(AfterPageHeader);
  return <>
    <header className={styles.header}>
      <div className={styles.copy}>
        {eyebrow && <span className={styles.eyebrow}>{eyebrow}</span>}
        <h1 className={styles.title}>{title}</h1>
        {subtitle && <p className={styles.subtitle}>{subtitle}</p>}
      </div>
      {actions && <div className={styles.actions}>{actions}</div>}
    </header>
    {after}
  </>;
}
