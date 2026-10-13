import { Film } from 'lucide-react';
import { Card } from '@/components/silicon-ui/card/card';
import { EmptyState } from '@/components/silicon-ui/empty-state/empty-state';
import { bytes, date } from '@/lib/format';
import { recordingStatusMessage } from '@/lib/recordings';
import type { Recording } from '@/lib/types';
import { ButtonRow, ExternalButtonLink, FinePrint, ListSection, PageHeader, StatusBadge, TaskButton } from '../ui';
import { useWorkspace } from '../workspace/WorkspaceContext';
import styles from './Recordings.module.css';

/** The recordings archive (view `recordings`), newest first. The workspace polls while any item is still recording or pending. */
export function Recordings() {
  const { recordings, navigate } = useWorkspace();
  return <div className={styles.screen}>
    <PageHeader
      eyebrow="Your archive"
      title="Recordings"
      subtitle="Revisit available session replays from your browser provider."
      actions={<TaskButton task={() => navigate('recordings')}>Refresh</TaskButton>}
    />
    <ListSection title="Your recordings">
      {recordings.length
        ? <ul className={styles.grid} aria-label="Recordings">
          {recordings.map(item => <li className={styles.item} key={item.session_id}><RecordingCard item={item} /></li>)}
        </ul>
        : <EmptyState icon={<Film size={20} strokeWidth={1.75} />} title="Nothing recorded yet" description="Available session recordings appear here." />}
    </ListSection>
  </div>;
}

/** `date · N min · size`, exactly as before. */
const summary = (item: Recording) => `${date(item.created_at)} · ${Math.round(item.duration_seconds / 60)} min · ${item.size_bytes ? bytes(item.size_bytes) : 'Size unavailable'}`;

function RecordingCard({ item }: { item: Recording }) {
  const { hideRecording } = useWorkspace();
  const message = recordingStatusMessage(item);
  return <Card className={styles.card} title={item.session_name} description={item.session_description}>
    <div className={styles.facts}>
      <StatusBadge status={item.status} />
      <span className={styles.summary}>{summary(item)}</span>
    </div>
    {message && <p className={styles.message} role="status">{message}</p>}
    <ButtonRow className={styles.actions}>
      <ExternalButtonLink href={item.briefcase_link}>Open recording</ExternalButtonLink>
      <TaskButton task={() => hideRecording(item)}>Hide</TaskButton>
    </ButtonRow>
    <FinePrint className={styles.fine}>Hiding removes this listing from Browser.</FinePrint>
  </Card>;
}
