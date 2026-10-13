import Head from 'next/head';
import { BrandMark } from '@/browser/shell/BrandMark';
import { ButtonLink } from '@/browser/ui/ButtonLink';
import styles from './404.module.css';

export default function NotFound() {
  return <>
    <Head><title>Page not found · Silicon Browser</title></Head>
    <main className={styles.page}>
      <a className={styles.brand} href="/"><BrandMark size={28} /><span>Browser</span></a>
      <p className={styles.code}>404</p>
      <h1 className={styles.title}>This page does not exist.</h1>
      <p className={styles.text}>Check the address, or return to your browser workspace.</p>
      <ButtonLink className={styles.home} href="/" variant="primary">Open Browser</ButtonLink>
    </main>
  </>;
}
