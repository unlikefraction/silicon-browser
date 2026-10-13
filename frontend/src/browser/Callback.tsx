import { useEffect, useState } from 'react';
import { Alert } from '@/components/silicon-ui/alert/alert';
import { acceptAuth, publicError, segment } from '@/lib/api';
import { clearRedirectAttempt, completeCallback, matchingRedirect } from '@/lib/auth';
import type { AuthSession } from '@/lib/types';
import { headline } from './Notices';
import { entry, productionApi } from './runtime';
import { Brand } from './shell/BrandMark';
import { ButtonLink } from './ui';
import styles from './Callback.module.css';

type Outcome = { message: string; failed: boolean };

// The one-use code is exchanged once per page, even when React StrictMode runs effects twice in development.
let started = false;

/** Finishes Accounts sign-in: hands the code to the opener (popup) or exchanges it in this tab (redirect). Never restores a saved session. */
async function complete(): Promise<Outcome | null> {
  if (!entry.callback || entry.error) return null;
  if (completeCallback(entry.callback)) return { message: 'Return to your Browser window to complete sign-in.', failed: false };
  if (!matchingRedirect(entry.callback) || !entry.callback.code || !entry.callback.attempt) return { message: 'This sign-in link is missing its original attempt. Start sign-in again from Browser.', failed: true };
  clearRedirectAttempt();
  try {
    const result = acceptAuth(await productionApi.request<AuthSession>(`/auth/attempts/${segment(entry.callback.attempt)}/exchange`, 'POST', { state: entry.callback.nonce, code: entry.callback.code, contract_version: 2 }, null), undefined, 'carbon');
    productionApi.setSession(result); clearRedirectAttempt(); location.replace('/');
    return null;
  } catch (error) { return { message: `${publicError(error)} Start sign-in again from Browser.`, failed: true }; }
}

export function Callback() {
  const [outcome, setOutcome] = useState<Outcome>({ message: entry.error || 'Returning to Browser…', failed: !!entry.error });
  useEffect(() => {
    if (started) return;
    started = true;
    void complete().then(result => { if (result) setOutcome(result); });
  }, []);
  const [title, details] = headline(outcome.message);
  return <main className={styles.callback}>
    <Brand className={styles.brand} link={false} />
    <h1 className={styles.title}>Sign in to Browser</h1>
    {outcome.failed
      ? <Alert tone="danger" title={title}>{details}</Alert>
      : <p className={styles.status} role="status">{outcome.message}</p>}
    <ButtonLink href="/" variant="primary">Open Browser</ButtonLink>
  </main>;
}
