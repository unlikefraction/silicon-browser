import { Skeleton } from '@/components/silicon-ui/skeleton/skeleton';
import { AddAccountPanel } from './AddAccountPanel';
import { AfterPageHeader } from './ui';
import { useWorkspace } from './workspace/WorkspaceContext';
import { Live } from './screens/Live';
import { NewSession } from './screens/NewSession';
import { ProfileForm } from './screens/ProfileForm';
import { Profiles } from './screens/Profiles';
import { Recordings } from './screens/Recordings';
import { SessionDetail } from './screens/SessionDetail';
import { Sessions } from './screens/Sessions';
import { Settings } from './screens/Settings';
import { Usage } from './screens/Usage';
import styles from './WorkspaceView.module.css';

/**
 * The signed-in view area: the loading skeleton while a view loads, otherwise the screen for the current view.
 * The "Add an account" panel (signed in only; signed out, the Welcome view is the sign-in) sits right after the screen's page header.
 */
export function WorkspaceView() {
  const { auth, addingContext, loading } = useWorkspace();
  const panel = auth && addingContext ? <AddAccountPanel /> : null;
  if (loading) return <>{panel}<div className={styles.loading}><Skeleton label="Loading workspace…" lines={4} /></div></>;
  return <AfterPageHeader.Provider value={panel}><Screen /></AfterPageHeader.Provider>;
}

function Screen() {
  const { view, session, profile } = useWorkspace();
  switch (view) {
    case 'sessions': return <Sessions />;
    case 'new-session': return <NewSession />;
    case 'detail': return session ? <SessionDetail session={session} /> : null;
    case 'live': return <Live />;
    case 'profiles': return <Profiles />;
    case 'new-profile': return <ProfileForm />;
    case 'edit-profile': return <ProfileForm profile={profile} />;
    case 'recordings': return <Recordings />;
    case 'usage': return <Usage />;
    case 'settings': return <Settings />;
  }
}
