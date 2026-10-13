import { useState, type FormEvent } from 'react';
import { AnimatePresence, motion, useReducedMotion, type Variants } from 'motion/react';
import { Button } from '@/components/silicon-ui/button/button';
import { CodeBlock } from '@/components/silicon-ui/code-block/code-block';
import { PasswordField } from '@/components/silicon-ui/password-field/password-field';
import { motionTokens } from '@/components/silicon-ui/lib/motion-tokens';
import { ButtonRow, FinePrint } from './ui';
import { useWorkspace } from './workspace/WorkspaceContext';
import styles from './SignInOptions.module.css';

const reveal: Variants = { open: { height: 'auto', opacity: 1 }, closed: { height: 0, opacity: 0 } };

/** "Continue as Carbon" (Accounts popup) and "Continue as Silicon" (short-lived token form). */
export function SignInOptions() {
  const { busy, signingIn, showSilicon, perform, login, siliconLogin, toggleSilicon } = useWorkspace();
  const reduced = useReducedMotion();
  /**
   * The token form opens and closes on its height, which needs clipping while it moves. It clips only during that motion, so the
   * keyboard focus outlines of "Sign in as Silicon" and the token field stay whole once it is open. A form already open when this
   * mounts does not animate in (AnimatePresence initial={false}), so it starts settled.
   */
  const [settled, setSettled] = useState(showSilicon);
  const submitToken = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    // The token is single-use: clear the field before the request, and never keep it anywhere else.
    event.currentTarget.reset();
    void perform(() => siliconLogin(data));
  };
  return <div className={styles.options}>
    <ButtonRow>
      <Button type="button" variant="primary" loading={signingIn} disabled={busy} onClick={() => void perform(login)}>Continue as Carbon</Button>
      <Button type="button" variant="secondary" disabled={busy} aria-expanded={showSilicon} onClick={toggleSilicon}>Continue as Silicon</Button>
    </ButtonRow>
    <AnimatePresence initial={false}>
      {showSilicon && <motion.div key="silicon" className={styles.reveal} data-settled={settled || undefined}
        variants={reveal} initial="closed" animate="open" exit="closed"
        onAnimationStart={() => setSettled(false)} onAnimationComplete={definition => setSettled(definition === 'open')}
        transition={reduced ? { duration: 0 } : { height: motionTokens.spring.smooth, opacity: { duration: motionTokens.duration.standard } }}>
        <form className={styles.form} autoComplete="off" onSubmit={submitToken}>
          <p className={styles.lead}>Get a short-lived token in your terminal:</p>
          <CodeBlock code="silicon-accounts login --app browser" filename="Terminal" language="bash" />
          <PasswordField label="Short-lived token" name="slt" required maxLength={16384} placeholder="slt_…" autoComplete="off" spellCheck={false} />
          <ButtonRow><Button type="submit" variant="primary" disabled={busy}>Sign in as Silicon</Button></ButtonRow>
          <FinePrint>A token works once and expires after two minutes.</FinePrint>
        </form>
      </motion.div>}
    </AnimatePresence>
  </div>;
}
