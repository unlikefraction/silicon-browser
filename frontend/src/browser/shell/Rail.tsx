import { AppWindow, ChartColumn, Clapperboard, Fingerprint, Plus, Settings, type LucideIcon } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { Avatar } from '@/components/silicon-ui/avatar/avatar';
import { Badge } from '@/components/silicon-ui/badge/badge';
import { Button } from '@/components/silicon-ui/button/button';
import { ScrollArea } from '@/components/silicon-ui/scroll-area/scroll-area';
import { motionTokens } from '@/components/silicon-ui/lib/motion-tokens';
import { contextKey } from '@/lib/session';
import { revealSignIn } from '../Welcome';
import { useWorkspace } from '../workspace/WorkspaceContext';
import { tabLabels, tabs, type Tab } from '../workspace/store';
import { Brand } from './BrandMark';
import styles from './Rail.module.css';

const icons: Record<Tab, LucideIcon> = { sessions: AppWindow, profiles: Fingerprint, recordings: Clapperboard, usage: ChartColumn, settings: Settings };

export interface RailContentProps {
  /** Distinguishes the desktop rail from the mobile drawer copy (shared-layout highlight ids). */
  id: string;
  /** Called after an account or tab button is pressed, e.g. to close the mobile drawer. */
  onChoose?: () => void;
  /** Hide the brand (the drawer has its own title). */
  showBrand?: boolean;
  /**
   * Signed out, "Add account" points at the Welcome sign-in panel instead of opening a second set of sign-in controls.
   * The drawer passes this to close itself first and reveal the panel once focus can move; the desktop rail reveals it at once.
   */
  onSignIn?: () => void;
}

/** Brand, saved accounts, workspace navigation, and the docs link. Shared by the desktop rail and the mobile drawer. */
export function RailContent({ id, onChoose, showBrand = true, onSignIn = revealSignIn }: RailContentProps) {
  const { auth, contexts, busy, activeTab, action, switchContext, addAccount, navigate } = useWorkspace();
  const reduced = useReducedMotion();
  const choose = (task: () => Promise<unknown>) => () => { action(task)(); onChoose?.(); };
  const listed = !!auth || contexts.length > 0;
  return <div className={styles.content}>
    {showBrand && <Brand className={styles.brand} />}
    {/* The viewport is always a tab stop (it scrolls with the keyboard), so it carries a name. */}
    <ScrollArea className={styles.scroll} viewportClassName={styles.viewport} label={listed ? 'Accounts and workspace' : 'About Browser'}>
      {listed ? <>
        <div className={styles.section} role="group" aria-labelledby={`${id}-accounts`}>
          <span className={styles.label} id={`${id}-accounts`}>Accounts</span>
          <ul className={styles.accounts}>
            {contexts.map(item => {
              const selected = !!auth && contextKey(auth) === contextKey(item);
              return <li key={contextKey(item)}>
                <button type="button" className={styles.account} aria-current={selected || undefined} disabled={busy} onClick={choose(() => switchContext(item))}>
                  <Avatar name={item.identity.name} size="sm" aria-hidden="true" />
                  <span className={styles.accountName}>{item.identity.name}</span>
                  <Badge size="sm" tone={selected ? 'info' : 'neutral'}>{item.identity.kind}</Badge>
                </button>
              </li>;
            })}
          </ul>
          <Button type="button" variant="ghost" size="sm" className={styles.add} disabled={busy} title={auth ? 'Sign in to another account' : 'Go to sign-in'} onClick={auth ? choose(addAccount) : onSignIn}>
            <Plus size={16} strokeWidth={1.75} aria-hidden="true" />Add account
          </Button>
        </div>
        {auth && <nav className={styles.section} aria-labelledby={`${id}-workspace`}>
          <span className={styles.label} id={`${id}-workspace`}>Workspace</span>
          <ul className={styles.nav}>
            {tabs.map(tab => {
              const Icon = icons[tab];
              const current = activeTab === tab;
              return <li key={tab}>
                <button type="button" className={styles.navItem} aria-current={current ? 'page' : undefined} disabled={busy} onClick={choose(() => navigate(tab))}>
                  {current && <motion.span className={styles.highlight} layoutId={`${id}-nav-highlight`} transition={reduced ? { duration: 0 } : motionTokens.spring.morph} aria-hidden="true" />}
                  <Icon className={styles.navIcon} size={16} strokeWidth={1.75} aria-hidden="true" />
                  <span className={styles.navLabel}>{tabLabels[tab]}</span>
                </button>
              </li>;
            })}
          </ul>
        </nav>}
      </> : <p className={styles.note}>A shared browser workspace for Carbons and Silicons.</p>}
    </ScrollArea>
    <div className={styles.bottom}>
      <a className={styles.docs} href="/docs/" target="_blank" rel="noopener noreferrer">CLI &amp; documentation <span aria-hidden="true">↗</span></a>
      <span className={styles.signature}>Team of Silicons</span>
    </div>
  </div>;
}
