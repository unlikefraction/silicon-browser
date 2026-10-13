import { ACCOUNTS_ORIGIN } from '@/lib/auth';
import { DefinitionList, ExternalButtonLink, FinePrint, PageHeader, Panel } from '../ui';
import { useWorkspace } from '../workspace/WorkspaceContext';
import styles from './Settings.module.css';

const kinds = { carbon: 'Carbon', silicon: 'Silicon' } as const;

/** The signed-in account and where to manage it. */
export function Settings() {
  const { auth } = useWorkspace();
  if (!auth) return null;
  return <div className={styles.screen}>
    <PageHeader eyebrow="Account" title="Settings" subtitle="Your Browser account." />
    <Panel narrow className={styles.panel} title="Signed in">
      <DefinitionList items={[
        { term: 'Account', value: auth.identity.name },
        { term: 'Account UUID', value: auth.identity.id, mono: true, copy: auth.identity.id },
        { term: 'Kind', value: kinds[auth.identity.kind] },
      ]} />
      <FinePrint>You stay signed in across tabs and browser restarts. Sign out to remove this account from this browser.</FinePrint>
      <div><ExternalButtonLink href={ACCOUNTS_ORIGIN}>Manage your account</ExternalButtonLink></div>
    </Panel>
  </div>;
}
