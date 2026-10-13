import { Panel, FinePrint } from './ui';
import { SignInOptions } from './SignInOptions';
import { useWorkspace } from './workspace/WorkspaceContext';
import styles from './Welcome.module.css';

const features = [
  { label: '01 / Profiles', text: 'Keep a consistent identity across sessions.' },
  { label: '02 / Together', text: 'Bring people and agents into the same browser.' },
  { label: '03 / Recorded', text: 'Return to your work when a session ends.' },
];

const SIGN_IN_ID = 'sign-in';

/**
 * Brings the Welcome sign-in panel into view and focuses its first enabled button. The rail's "Add account" does this when signed
 * out, so the page never shows a second set of sign-in controls.
 */
export function revealSignIn() {
  const panel = document.getElementById(SIGN_IN_ID);
  if (!panel) return;
  const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  panel.scrollIntoView({ block: 'start', behavior: reduce ? 'auto' : 'smooth' });
  panel.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true });
}

/** The signed-out landing view with both sign-in paths. */
export function Welcome() {
  const { pendingInvitation } = useWorkspace();
  return <section className={styles.welcome} aria-labelledby="welcome-title">
    <span className={styles.eyebrow}>Your browser workspace</span>
    <h1 className={styles.title} id="welcome-title">A browser, ready<br />when you are.</h1>
    <p className={styles.lead}>Start a session. Keep your profiles. Work together across the web.</p>
    <Panel id={SIGN_IN_ID} className={styles.signIn} title="Sign in to Browser" description="Use your Silicon Accounts identity.">
      <SignInOptions />
      <FinePrint>Carbon sign-in opens in a separate window, or continues in this tab when popups are blocked.</FinePrint>
    </Panel>
    {pendingInvitation && <p className={styles.hint} role="status">You have a live browser invitation. Sign in to continue.</p>}
    <ul className={styles.features}>
      {features.map(feature => <li key={feature.label}><span className={styles.featureLabel}>{feature.label}</span><p>{feature.text}</p></li>)}
    </ul>
  </section>;
}
