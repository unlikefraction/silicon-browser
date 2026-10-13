import { AppWindow, Plus } from 'lucide-react';
import { EmptyState } from '@/components/silicon-ui/empty-state/empty-state';
import { SearchField } from '@/components/silicon-ui/search-field/search-field';
import { SortableDataTable, type DataColumn } from '@/components/silicon-ui/sortable-data-table/sortable-data-table';
import { cost, date } from '@/lib/format';
import type { Session } from '@/lib/types';
import { ListSection, PageHeader, StatusBadge, TaskButton, Toolbar } from '../ui';
import { useWorkspace } from '../workspace/WorkspaceContext';
import styles from './Sessions.module.css';

/** A table row: the session plus plain sort keys (start time and usage cost), since the table sorts on the raw cell value. */
const toRow = (session: Session) => ({
  ...session,
  started: Date.parse(session.started_at),
  cost: session.usage?.cost?.total?.micros,
});
type SessionRow = ReturnType<typeof toRow>;

/** The sessions list (view `sessions`): search, refresh, a sortable table, and an empty state. */
export function Sessions() {
  const { busy, filter, setFilter, matchingSessions, navigate, newSession, detail } = useWorkspace();

  const columns: DataColumn<SessionRow>[] = [
    {
      key: 'name',
      label: 'Session',
      render: (_, row) => <span className={styles.session}>
        <strong className={styles.name}>{row.name}</strong>
        <span className={styles.id}>{row.id}</span>
      </span>,
    },
    { key: 'status', label: 'Status', render: (_, row) => <StatusBadge status={row.status} /> },
    { key: 'started', label: 'Started', numeric: false, render: (_, row) => date(row.started_at) },
    { key: 'cost', label: 'Usage', numeric: true, render: (_, row) => cost(row.usage) },
    {
      key: 'actions',
      label: 'Actions',
      sortable: false,
      render: (_, row) => <TaskButton size="sm" aria-label={`Open ${row.name}`} task={() => detail(row.id)}>Open</TaskButton>,
    },
  ];

  return <div className={styles.screen}>
    <PageHeader
      eyebrow="Workspace"
      title="Sessions"
      subtitle="Your work across the web, in one place."
      actions={<TaskButton variant="primary" task={() => newSession()}>
        <Plus size={16} strokeWidth={1.75} aria-hidden="true" />New session
      </TaskButton>}
    />
    <Toolbar search={<SearchField label="Find a session" placeholder="Find a session…" value={filter} onValueChange={setFilter} disabled={busy} />}>
      <TaskButton task={() => navigate('sessions')}>Refresh</TaskButton>
    </Toolbar>
    <ListSection title="All sessions">
      {matchingSessions.length
        ? <div className={styles.table}>
          <SortableDataTable rows={matchingSessions.map(toRow)} columns={columns} rowKey="id" caption="Sessions" />
        </div>
        : <EmptyState
          icon={<AppWindow size={20} strokeWidth={1.75} />}
          title="No sessions yet"
          description="Start with a fresh browser, or choose a saved profile."
          action={<TaskButton variant="primary" task={() => newSession()}>Start your first session</TaskButton>}
        />}
    </ListSection>
  </div>;
}
