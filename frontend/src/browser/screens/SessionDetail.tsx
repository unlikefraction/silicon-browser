import { MonitorPlay } from 'lucide-react';
import { CodeBlock } from '@/components/silicon-ui/code-block/code-block';
import { Textarea } from '@/components/silicon-ui/textarea/textarea';
import { cost, date } from '@/lib/format';
import type { Session } from '@/lib/types';
import { ButtonRow, DefinitionList, FormStack, PageHeader, Panel, StatusBadge, SubmitButton, TaskButton } from '../ui';
import { useWorkspace } from '../workspace/WorkspaceContext';
import styles from './SessionDetail.module.css';

/** The initiator first, then every other participant once. */
const participants = (session: Session) => [session.initiator_id, ...(session.participant_ids || [])]
  .filter((value, index, list) => list.indexOf(value) === index)
  .join(', ');

/** One session (view `detail`). Rendered only when the workspace has a session. */
export function SessionDetail({ session }: { session: Session }) {
  const { busy, submit, endSession, openLive, detail } = useWorkspace();
  const active = session.status === 'active';
  return <div className={styles.screen}>
    <PageHeader eyebrow="Session" title={session.name} subtitle={session.description} actions={<StatusBadge status={session.status} size="md" />} />
    <Panel>
      <DefinitionList items={[
        { term: 'Session ID', value: session.id, mono: true, copy: session.id },
        { term: 'Started', value: date(session.started_at) },
        { term: 'Ends by', value: date(session.expires_at) },
        { term: 'Usage', value: cost(session.usage) },
        { term: 'Participants', value: participants(session), mono: true },
      ]} />
      <ButtonRow>
        {active && <TaskButton variant="primary" task={() => openLive(session.id)}>
          <MonitorPlay size={16} strokeWidth={1.75} aria-hidden="true" />Open live browser
        </TaskButton>}
        <TaskButton task={() => detail(session.id)}>Refresh</TaskButton>
      </ButtonRow>
    </Panel>
    <Panel tone="subtle" title="Continue in your terminal" description="Use the same session from the Browser CLI.">
      <CodeBlock filename="Terminal" language="bash" code={`browser run ${session.id} snapshot`} />
    </Panel>
    {active && <Panel narrow title="End session">
      <FormStack onSubmit={submit(endSession)}>
        <Textarea name="note" label="Closing note" required maxLength={4000} placeholder="What did you finish?" disabled={busy} />
        <ButtonRow><SubmitButton variant="danger">End session</SubmitButton></ButtonRow>
      </FormStack>
    </Panel>}
  </div>;
}
