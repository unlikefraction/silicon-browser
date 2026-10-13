import type { FormHTMLAttributes, HTMLAttributes, ReactNode } from 'react';
import styles from './Stack.module.css';

/** A vertical form: fields, fine print, and the submit row, evenly spaced. Pass onSubmit={submit(task)} from useWorkspace. */
export function FormStack({ className, children, ...props }: FormHTMLAttributes<HTMLFormElement>) {
  return <form {...props} className={[styles.stack, className].filter(Boolean).join(' ')}>{children}</form>;
}

/** A row of buttons or links that wraps on narrow screens. */
export function ButtonRow({ className, children, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div {...props} className={[styles.row, className].filter(Boolean).join(' ')}>{children}</div>;
}

/** Small secondary text: hints, caveats, and footnotes. */
export function FinePrint({ className, children, ...props }: HTMLAttributes<HTMLParagraphElement> & { children?: ReactNode }) {
  return <p {...props} className={[styles.fine, className].filter(Boolean).join(' ')}>{children}</p>;
}
