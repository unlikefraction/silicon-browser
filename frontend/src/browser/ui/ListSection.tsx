import { useId, type ReactNode } from 'react';
import styles from './ListSection.module.css';

/**
 * A region with an <h2> for assistive technology only. Card and EmptyState titles are <h3>, so a list placed directly under the
 * page's <h1> would skip a heading level; this keeps the outline in order without adding visible chrome.
 */
export function ListSection({ title, className, children }: { title: string; className?: string; children: ReactNode }) {
  const id = useId();
  return <section className={className} aria-labelledby={id}>
    <h2 className={styles.title} id={id}>{title}</h2>
    {children}
  </section>;
}
