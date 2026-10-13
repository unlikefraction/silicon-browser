import type { ReactNode } from 'react';
import styles from './Toolbar.module.css';

/** A row above a list: the search field grows, actions sit at the end. */
export function Toolbar({ search, children }: { search?: ReactNode; children?: ReactNode }) {
  return <div className={styles.toolbar}>
    {search && <div className={styles.search}>{search}</div>}
    {children && <div className={styles.actions}>{children}</div>}
  </div>;
}
