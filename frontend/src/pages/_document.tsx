import { Head, Html, Main, NextScript } from 'next/document';

// No inline scripts or styles: the production CSP is `script-src 'self'`. boot.js must stay the first, synchronous script in <head>.
export default function Document() {
  return <Html lang="en" data-accent="neutral" data-corners="squircle">
    <Head>
      {/* eslint-disable-next-line @next/next/no-sync-scripts */}
      <script src="/boot.js" />
      <meta name="referrer" content="no-referrer" />
      <meta name="description" content="Your browsers, profiles, and recordings." />
      <link rel="icon" type="image/svg+xml" href="/mark.svg" />
    </Head>
    <body>
      <Main />
      <NextScript />
    </body>
  </Html>;
}
