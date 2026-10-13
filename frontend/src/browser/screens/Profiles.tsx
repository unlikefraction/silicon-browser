import { Fingerprint, Plus } from 'lucide-react';
import { Button } from '@/components/silicon-ui/button/button';
import { Card } from '@/components/silicon-ui/card/card';
import { EmptyState } from '@/components/silicon-ui/empty-state/empty-state';
import { SearchField } from '@/components/silicon-ui/search-field/search-field';
import type { Profile } from '@/lib/types';
import { ButtonRow, ListSection, PageHeader, StatusBadge, TaskButton, Toolbar } from '../ui';
import { useWorkspace } from '../workspace/WorkspaceContext';
import styles from './Profiles.module.css';

/** The profiles grid (view `profiles`): search, refresh, a card per profile, and an empty state. */
export function Profiles() {
  const { busy, filter, setFilter, matchingProfiles, navigate, openNewProfile } = useWorkspace();
  return <div className={styles.screen}>
    <PageHeader
      eyebrow="Persistent identities"
      title="Profiles"
      subtitle="A familiar browser, every time you return."
      actions={<Button type="button" variant="primary" disabled={busy} onClick={openNewProfile}>
        <Plus size={16} strokeWidth={1.75} aria-hidden="true" />New profile
      </Button>}
    />
    <Toolbar search={<SearchField label="Find a profile" placeholder="Find a profile…" value={filter} onValueChange={setFilter} disabled={busy} />}>
      <TaskButton task={() => navigate('profiles')}>Refresh</TaskButton>
    </Toolbar>
    <ListSection title="Your profiles">
      {matchingProfiles.length
        ? <ul className={styles.grid} aria-label="Profiles">
          {matchingProfiles.map(item => <li className={styles.item} key={item.id}><ProfileCard item={item} /></li>)}
        </ul>
        : <EmptyState
          icon={<Fingerprint size={20} strokeWidth={1.75} />}
          title="No profiles yet"
          description="Create a profile to keep your browser identity and location between sessions."
        />}
    </ListSection>
  </div>;
}

/** One profile: location and status, who owns it and who else can use it, and (while active) its actions. */
function ProfileCard({ item }: { item: Profile }) {
  const { busy, newSession, editProfile } = useWorkspace();
  return <Card className={styles.card} title={item.name} description={`${item.location.name} · ${item.sessions_run} sessions`}>
    <div className={styles.facts}>
      <span className={styles.code}><span className={styles.srOnly}>Location code </span>{item.location.code.toUpperCase()}</span>
      <StatusBadge status={item.status} />
    </div>
    <dl className={styles.people}>
      <div className={styles.row}><dt>Owner</dt><dd className={styles.mono}>{item.owner_id}</dd></div>
      <div className={styles.row}><dt>Access</dt><dd className={item.access.length ? styles.mono : undefined}>{item.access.join(', ') || 'Owner only'}</dd></div>
    </dl>
    {item.status === 'active' && <ButtonRow className={styles.actions}>
      <TaskButton variant="primary" aria-label={`Start session with ${item.name}`} task={() => newSession(item)}>Start session</TaskButton>
      <Button type="button" variant="secondary" aria-label={`Edit ${item.name}`} disabled={busy} onClick={() => editProfile(item)}>Edit</Button>
    </ButtonRow>}
  </Card>;
}
