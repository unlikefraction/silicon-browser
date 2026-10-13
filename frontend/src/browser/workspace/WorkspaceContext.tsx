import { createContext, useContext, useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from 'react';
import { useToastStack } from '@/components/silicon-ui/toast-stack/toast-stack';
import { entry, productionApi, restored, savedSession } from '../runtime';
import { WorkspaceStore, activeTab, type Tab, type WorkspaceState } from './store';

const WorkspaceContext = createContext<WorkspaceStore | null>(null);

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const [store] = useState(() => new WorkspaceStore({ api: productionApi, saved: savedSession, entry, restored }));
  const { toast } = useToastStack();
  useEffect(() => { store.toast = options => { toast(options); }; }, [store, toast]);
  useEffect(() => store.start(), [store]);
  return <WorkspaceContext.Provider value={store}>{children}</WorkspaceContext.Provider>;
}

function bind(store: WorkspaceStore) {
  return {
    perform: store.perform,
    action: store.action,
    submit: store.submit,
    navigate: store.navigate,
    detail: store.detail,
    openLive: store.openLive,
    newSession: store.newSession,
    openNewProfile: store.openNewProfile,
    editProfile: store.editProfile,
    refreshRecordings: store.refreshRecordings,
    createSession: store.createSession,
    endSession: store.endSession,
    saveProfile: store.saveProfile,
    retireProfile: store.retireProfile,
    hideRecording: store.hideRecording,
    login: store.login,
    cancelSignIn: store.cancelSignIn,
    siliconLogin: store.siliconLogin,
    toggleSilicon: store.toggleSilicon,
    switchContext: store.switchContext,
    addAccount: store.addAccount,
    closeAddAccount: store.closeAddAccount,
    logout: store.logout,
    setFilter: store.setFilter,
    dismissNotice: store.dismissNotice,
    toast: (options: Parameters<WorkspaceStore['toast']>[0]) => store.toast(options),
  };
}

export type WorkspaceActions = ReturnType<typeof bind>;
export interface WorkspaceDerived {
  /** The rail tab for the current view (detail/live/new-session → sessions; profile forms → profiles). */
  activeTab: Tab;
  /** Sessions whose `name id status` contains the filter (case-insensitive). */
  matchingSessions: WorkspaceState['sessions'];
  /** Profiles whose `name id location.name` contains the filter (case-insensitive). */
  matchingProfiles: WorkspaceState['profiles'];
}
export type Workspace = WorkspaceState & WorkspaceActions & WorkspaceDerived;

/** The workspace state, actions, and derived lists. Re-renders on every workspace change. */
export function useWorkspace(): Workspace {
  const store = useContext(WorkspaceContext);
  if (!store) throw new Error('useWorkspace must be used inside <WorkspaceProvider>.');
  const state = useSyncExternalStore(store.subscribe, store.getState, store.getState);
  const actions = useMemo(() => bind(store), [store]);
  return useMemo(() => {
    const filter = state.filter.toLowerCase();
    return {
      ...state,
      ...actions,
      activeTab: activeTab(state.view),
      matchingSessions: state.sessions.filter(item => `${item.name} ${item.id} ${item.status}`.toLowerCase().includes(filter)),
      matchingProfiles: state.profiles.filter(item => `${item.name} ${item.id} ${item.location.name}`.toLowerCase().includes(filter)),
    };
  }, [state, actions]);
}
