// Client-only entry (pages/index.tsx loads it with ssr: false). Importing runtime parses the captured entry URL once.
import { App } from './App';
import { Callback } from './Callback';
import { entry } from './runtime';

export default function Root() {
  return entry.callback ? <Callback /> : <App />;
}
