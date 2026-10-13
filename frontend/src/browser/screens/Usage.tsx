import { Gauge } from 'lucide-react';
import { AnimatedCounter } from '@/components/silicon-ui/animated-counter/animated-counter';
import { EmptyState } from '@/components/silicon-ui/empty-state/empty-state';
import { SortableDataTable, type DataColumn } from '@/components/silicon-ui/sortable-data-table/sortable-data-table';
import { cost, date, minutes } from '@/lib/format';
import type { Usage as UsageRow } from '@/lib/types';
import { ListSection, PageHeader, TaskButton } from '../ui';
import { useWorkspace } from '../workspace/WorkspaceContext';
import styles from './Usage.module.css';

/** The counter lays its suffix out as its own box, where a plain leading space would collapse. */
const NBSP = ' ';

/** Table rows are plain objects; `micros` gives the Usage column a numeric sort (Pending rows stay last). */
const toRow = (row: UsageRow, index: number) => ({ ...row, key: row.session_id ?? `#${index}`, micros: row.cost?.total?.micros });
type Row = ReturnType<typeof toRow>;

const columns: DataColumn<Row>[] = [
  { key: 'session_id', label: 'Session', render: (_, row) => <span className={styles.id}>{row.session_id}</span> },
  { key: 'browser_seconds', label: 'Browser time', numeric: true, render: (_, row) => `${minutes(row.browser_seconds)} min` },
  { key: 'micros', label: 'Usage', numeric: true, render: (_, row) => cost(row) },
];

/** Usage totals, the shared service capacity, and per-session usage (view `usage`). */
export function Usage() {
  const { usage, total, limits, navigate } = useWorkspace();
  return <div className={styles.screen}>
    <PageHeader
      eyebrow="Workspace activity"
      title="Usage"
      subtitle="Browser time and network usage for sessions you can access."
      actions={<TaskButton task={() => navigate('usage')}>Refresh</TaskButton>}
    />
    {total && <Totals total={total} />}
    <p className={styles.capacity}>
      {limits
        ? <><strong className={styles.limit}>{limits.concurrent_browser_limit} concurrent browsers</strong> · Shared service limit · Checked {date(limits.checked_at)}</>
        : 'Service capacity is temporarily unavailable.'}
    </p>
    <ListSection title="Usage by session">
      {usage.length
        ? <div className={styles.table}><SortableDataTable caption="Usage by session" rows={usage.map(toRow)} rowKey="key" columns={columns} /></div>
        : <EmptyState icon={<Gauge size={20} strokeWidth={1.75} />} title="No session usage yet" description="Usage appears here once a session opens a browser." />}
    </ListSection>
  </div>;
}

function Totals({ total }: { total: UsageRow }) {
  const traffic = trafficParts(total.proxy_bytes_in + total.proxy_bytes_out + (total.proxy_bytes_unclassified || 0));
  return <dl className={styles.metrics}>
    <Metric label="Browser time" value={Number(minutes(total.browser_seconds))} decimals={1} suffix={`${NBSP}min`} context="Open browser time across these sessions" />
    <Metric label="Proxy traffic" {...traffic} context="Data in and out through the proxy" />
    <Metric label="Visible sessions" value={total.sessions ?? 0} context="Sessions you can access" />
  </dl>;
}

/** The same unit choice and rounding as `bytes()`: GB with 2 decimals from 1e9 bytes, otherwise MB with 1. */
function trafficParts(bytes: number) {
  return bytes >= 1e9
    ? { value: Number((bytes / 1e9).toFixed(2)), decimals: 2, suffix: `${NBSP}GB` }
    : { value: Number((bytes / 1e6).toFixed(1)), decimals: 1, suffix: `${NBSP}MB` };
}

/**
 * Silicon UI's MetricCard look, built on its AnimatedCounter. MetricCard always rounds to a whole number,
 * which would turn "12.3 min" into "12 min" and "1.23 GB" into "1 GB"; the counter keeps the decimals shown before.
 */
function Metric({ label, value, decimals = 0, suffix, context }: { label: string; value: number; decimals?: number; suffix?: string; context: string }) {
  return <div className={styles.metric}>
    <dt className={styles.label}>{label}</dt>
    <dd className={styles.value}><AnimatedCounter value={value} decimals={decimals} suffix={suffix} animateOnView /></dd>
    <dd className={styles.context}>{context}</dd>
  </div>;
}
