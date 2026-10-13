import dynamic from 'next/dynamic';

// Nothing user-specific is server-rendered: the app reads the entry URL, storage, and tokens only in the browser.
const Root = dynamic(() => import('../browser/Root'), { ssr: false });

export default function Index() {
  return <Root />;
}
