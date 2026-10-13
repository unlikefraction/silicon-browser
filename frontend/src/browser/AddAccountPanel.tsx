import { useEffect } from 'react';
import { Button } from '@/components/silicon-ui/button/button';
import { Panel } from './ui';
import { SignInOptions } from './SignInOptions';
import { useWorkspace } from './workspace/WorkspaceContext';
import styles from './AddAccountPanel.module.css';

const PANEL_ID = 'add-account';

/**
 * Signs in one more account without signing out the current one. Signed in only (WorkspaceView renders it after the page header);
 * signed out, the Welcome view is the one place with sign-in controls.
 */
export function AddAccountPanel() {
  const { busy, closeAddAccount } = useWorkspace();
  // "Add account" can be pressed with the page scrolled down (or from the phone drawer): bring the panel into view if it is not.
  useEffect(() => {
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    document.getElementById(PANEL_ID)?.scrollIntoView({ block: 'nearest', behavior: reduce ? 'auto' : 'smooth' });
  }, []);
  return <Panel id={PANEL_ID} className={styles.panel} narrow title="Add an account" description="Each account keeps its own sessions, profiles, and sign-in.">
    <SignInOptions />
    <div><Button type="button" variant="ghost" disabled={busy} onClick={closeAddAccount}>Cancel</Button></div>
  </Panel>;
}
