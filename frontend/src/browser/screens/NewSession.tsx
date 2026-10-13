import { Input } from '@/components/silicon-ui/input/input';
import { MorphSelect, type MorphSelectOption } from '@/components/silicon-ui/morph-select/morph-select';
import { Textarea } from '@/components/silicon-ui/textarea/textarea';
import { ButtonRow, FinePrint, FormStack, PageHeader, Panel, SubmitButton, TaskButton } from '../ui';
import { useWorkspace } from '../workspace/WorkspaceContext';
import styles from './NewSession.module.css';

/** Session lengths offered by the form; the value is the `ttl` sent to the backend. */
const lengths: MorphSelectOption[] = [15, 30, 45, 60, 120, 240].map(minutes => ({ value: `${minutes}m`, label: `${minutes} minutes` }));

/** The new-session form (view `new-session`); `profile` in the workspace is the profile it starts from, if any. */
export function NewSession() {
  const { busy, profile, submit, createSession, navigate } = useWorkspace();
  return <div className={styles.screen}>
    <PageHeader
      eyebrow="Workspace"
      title={profile ? `Start ${profile.name}` : 'New session'}
      subtitle={profile ? 'Continue with your saved browser identity.' : 'A fresh browser with no saved profile.'}
      actions={<TaskButton task={() => navigate('sessions')}>Back to sessions</TaskButton>}
    />
    <Panel narrow>
      <FormStack onSubmit={submit(createSession)}>
        <Input name="name" label="Session name" required maxLength={120} placeholder="What are you working on?" disabled={busy} />
        <Textarea name="description" label="Description" required maxLength={2000} placeholder="A little context for you and your collaborators." disabled={busy} />
        {/* MorphSelect has no `required`; the default keeps a value selected, like the former native select. */}
        <MorphSelect name="ttl" label="Session length" items={lengths} defaultValue="15m" searchable={false} disabled={busy} />
        <FinePrint>Usage starts when the browser opens. The session ends automatically at its time limit.</FinePrint>
        <ButtonRow><SubmitButton>Start session</SubmitButton></ButtonRow>
      </FormStack>
    </Panel>
  </div>;
}
