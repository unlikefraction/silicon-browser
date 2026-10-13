import { useRef } from 'react';
import { Alert } from '@/components/silicon-ui/alert/alert';
import { Button } from '@/components/silicon-ui/button/button';
import { useWorkspace } from './workspace/WorkspaceContext';
import styles from './Notices.module.css';

/** Splits "First sentence. The rest." so the alert can lead with the first sentence. */
export function headline(message: string): [string, string | undefined] {
  const match = message.match(/^(.+?[.!?])\s+(\S[\s\S]*)$/);
  return match ? [match[1]!, match[2]] : [message, undefined];
}

/** The workspace notice and the Carbon sign-in wait banner, inside a polite live region. */
export function Notices() {
  const { notice, signingIn, cancelSignIn, dismissNotice } = useWorkspace();
  // Keep the last message while the alert animates out.
  const shown = useRef(notice);
  if (notice) shown.current = notice;
  const [title, details] = headline(shown.current);
  return <div className={styles.notices} aria-live="polite">
    <Alert open={!!notice} tone="danger" title={title} onDismiss={dismissNotice}>{details}</Alert>
    <Alert open={signingIn} tone="info" title="Complete sign-in in the Silicon Accounts window.">
      {/* Never disabled by busy: this is how a pending sign-in is abandoned. */}
      <Button type="button" variant="secondary" size="sm" className={styles.cancel} onClick={cancelSignIn}>Cancel sign-in</Button>
    </Alert>
  </div>;
}
