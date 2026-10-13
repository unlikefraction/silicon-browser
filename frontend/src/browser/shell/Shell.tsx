import { ToastStack } from '@/components/silicon-ui/toast-stack/toast-stack';
import { Notices } from '../Notices';
import { Welcome } from '../Welcome';
import { WorkspaceView } from '../WorkspaceView';
import { useTheme } from '../useTheme';
import { useWorkspace } from '../workspace/WorkspaceContext';
import { Footer } from './Footer';
import { RailContent } from './Rail';
import { TopBar } from './TopBar';
import styles from './Shell.module.css';

/** Rail, top bar, notices, the current view, and the footer. */
export function Shell() {
  const { auth, view } = useWorkspace();
  useTheme(); // Follows system theme changes for the whole page.
  return <div className={styles.shell}>
    {/* The aside stretches with the page (background and border); its content stays pinned to the viewport. */}
    <aside className={styles.rail} aria-label="Browser">
      <div className={styles.railContent}><RailContent id="rail" /></div>
    </aside>
    <div className={styles.column}>
      <TopBar />
      <main className={styles.main} data-live={view === 'live' && !!auth ? '' : undefined}>
        <Notices />
        {/* Signed out, Welcome holds the only sign-in controls; signed in, WorkspaceView places "Add an account" after the page header. */}
        {auth ? <WorkspaceView /> : <Welcome />}
      </main>
      <Footer />
    </div>
    <ToastStack />
  </div>;
}
