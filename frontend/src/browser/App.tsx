import { Shell } from './shell/Shell';
import { WorkspaceProvider } from './workspace/WorkspaceContext';

export function App() {
  return <WorkspaceProvider><Shell /></WorkspaceProvider>;
}
