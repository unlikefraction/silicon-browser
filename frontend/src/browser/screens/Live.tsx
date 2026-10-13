import { useRef } from 'react';
import { Maximize2, MonitorPlay } from 'lucide-react';
import { EmptyState } from '@/components/silicon-ui/empty-state/empty-state';
import { ListSection, PageHeader, TaskButton } from '../ui';
import { useWorkspace } from '../workspace/WorkspaceContext';
import styles from './Live.module.css';

/** The redeemed live browser in a sandboxed iframe. The iframe attributes are a security contract: keep them exactly. */
export function Live() {
  const { session, liveUrl, detail } = useWorkspace();
  const frame = useRef<HTMLIFrameElement>(null);
  return <div className={styles.live}>
    <PageHeader
      eyebrow="Live session"
      title={session?.name || 'Live browser'}
      subtitle="Work in this browser together. The original session time limit still applies."
      actions={<>
        {document.fullscreenEnabled && liveUrl && <TaskButton task={async () => { if (frame.current) await frame.current.requestFullscreen(); }}>
          <Maximize2 size={16} strokeWidth={1.75} aria-hidden="true" />Fullscreen
        </TaskButton>}
        {session && <TaskButton task={() => detail(session.id)}>Session details</TaskButton>}
      </>}
    />
    {liveUrl
      ? <div className={styles.viewer}>
        <iframe ref={frame} className={styles.frame} title="Live browser session" src={liveUrl} referrerPolicy="no-referrer" sandbox="allow-scripts allow-same-origin allow-forms allow-popups" allow="clipboard-read; clipboard-write; fullscreen" />
      </div>
      : <ListSection title="Live browser">
        <EmptyState icon={<MonitorPlay size={20} strokeWidth={1.75} />} title="Live browser unavailable" description="The live browser is unavailable. Refresh the session to check its status." />
      </ListSection>}
  </div>;
}
