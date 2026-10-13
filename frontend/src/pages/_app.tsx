import type { AppProps } from 'next/app';
import Head from 'next/head';
import '@/browser/styles/fonts.css';
import '@/components/silicon-ui/foundation.css';
import '@/browser/styles/global.css';
import { ToastStackProvider } from '@/components/silicon-ui/toast-stack/toast-stack';

export default function BrowserApp({ Component, pageProps }: AppProps) {
  return <>
    <Head>
      <meta name="viewport" content="width=device-width, initial-scale=1" />
      <title>Silicon Browser</title>
    </Head>
    <ToastStackProvider>
      <Component {...pageProps} />
    </ToastStackProvider>
  </>;
}
