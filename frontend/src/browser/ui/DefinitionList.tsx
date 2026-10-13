import type { ReactNode } from 'react';
import { CopyButton } from '@/components/silicon-ui/copy-button/copy-button';
import styles from './DefinitionList.module.css';

export interface DefinitionItem {
  term: string;
  value: ReactNode;
  /** Monospace value, for ids and UUIDs. */
  mono?: boolean;
  /** Adds an icon-only CopyButton for this text, labelled "Copy <term>". */
  copy?: string;
}

/** Term/value pairs in a responsive two-column grid (stacks on narrow screens). */
export function DefinitionList({ items }: { items: DefinitionItem[] }) {
  return <dl className={styles.list}>
    {items.map(item => <div className={styles.row} key={item.term}>
      <dt className={styles.term}>{item.term}</dt>
      <dd className={styles.value}>
        <span className={item.mono ? styles.mono : undefined}>{item.value}</span>
        {item.copy && <CopyButton value={item.copy} label={`Copy ${item.term}`} iconOnly variant="plain" />}
      </dd>
    </div>)}
  </dl>;
}
