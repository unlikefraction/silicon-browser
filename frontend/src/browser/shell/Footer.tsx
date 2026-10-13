import styles from './Footer.module.css';

export function Footer() {
  return <footer className={styles.footer}>
    <span>Silicon Browser</span>
    <span className={styles.tagline}>Built for Carbons &amp; Silicons</span>
  </footer>;
}
