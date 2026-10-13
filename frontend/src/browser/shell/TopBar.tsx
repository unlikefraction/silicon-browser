import { useEffect, useRef, useState } from 'react';
import { ExternalLink, Menu, Settings } from 'lucide-react';
import { Breadcrumb } from '@/components/silicon-ui/breadcrumb/breadcrumb';
import { Button } from '@/components/silicon-ui/button/button';
import { Drawer, DrawerContent, DrawerTrigger } from '@/components/silicon-ui/drawer/drawer';
import { UserMenu } from '@/components/silicon-ui/user-menu/user-menu';
import { safeHttps } from '@/lib/api';
import { ACCOUNTS_ORIGIN } from '@/lib/auth';
import { revealSignIn } from '../Welcome';
import { useTheme } from '../useTheme';
import { useWorkspace } from '../workspace/WorkspaceContext';
import { tabLabels } from '../workspace/store';
import { Brand } from './BrandMark';
import { RailContent } from './Rail';
import styles from './TopBar.module.css';

const kindLabels = { carbon: 'Carbon account', silicon: 'Silicon account' } as const;
/** Where the rail is beside the page again (Shell.module.css and TopBar.module.css collapse it at max-width: 720px). */
const RAIL_SHOWN = '(min-width: 721px)';

/** Opens an https page in a new tab without opener or referrer. */
function openExternal(href: string) {
  const url = safeHttps(href);
  if (url) window.open(url, '_blank', 'noopener,noreferrer');
}

/** Breadcrumb, the account menu, and (below 720px) the menu button that opens the rail in a drawer. */
export function TopBar() {
  const { auth, busy, activeTab, action, navigate, logout } = useWorkspace();
  const { preference, setPreference } = useTheme();
  const [menuOpen, setMenuOpen] = useState(false);
  /** Set when the drawer closes for "Add account" while signed out: once the drawer has let go of focus, reveal the sign-in panel. */
  const signInAfterClose = useRef(false);
  // The drawer only exists for the collapsed rail: close it when the window grows wide enough to show the rail again.
  useEffect(() => {
    const wide = window.matchMedia(RAIL_SHOWN);
    const close = () => { if (wide.matches) setMenuOpen(false); };
    wide.addEventListener('change', close);
    return () => wide.removeEventListener('change', close);
  }, []);
  return <header className={styles.topbar}>
    <div className={styles.start}>
      <div className={styles.mobile}>
        <Drawer open={menuOpen} onOpenChange={setMenuOpen}>
          <DrawerTrigger asChild>
            <Button type="button" variant="ghost" size="sm" aria-label="Open navigation"><Menu size={18} strokeWidth={1.75} aria-hidden="true" /></Button>
          </DrawerTrigger>
          <DrawerContent side="left" title="Browser" description="Accounts and workspace" className={styles.drawer}
            onCloseAutoFocus={event => { if (!signInAfterClose.current) return; signInAfterClose.current = false; event.preventDefault(); revealSignIn(); }}>
            <RailContent id="drawer" showBrand={false} onChoose={() => setMenuOpen(false)} onSignIn={() => { signInAfterClose.current = true; setMenuOpen(false); }} />
          </DrawerContent>
        </Drawer>
        <Brand className={styles.brand} />
      </div>
      <Breadcrumb items={[{ label: 'Browser' }, { label: auth ? tabLabels[activeTab] : 'Welcome' }]} />
    </div>
    {auth && <fieldset className={styles.account} disabled={busy}>
      <legend className={styles.legend}>Account</legend>
      <UserMenu
        user={{ name: auth.identity.name, email: kindLabels[auth.identity.kind] }}
        items={[
          { label: 'Settings', icon: <Settings size={16} strokeWidth={1.75} />, onSelect: action(() => navigate('settings')) },
          { label: 'Manage account', icon: <ExternalLink size={16} strokeWidth={1.75} />, onSelect: () => openExternal(ACCOUNTS_ORIGIN) },
        ]}
        onSignOut={logout}
        showTheme
        theme={preference}
        onThemeChange={setPreference}
        showName
      />
    </fieldset>}
  </header>;
}
