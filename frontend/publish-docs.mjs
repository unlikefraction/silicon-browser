// Ship the repository's Markdown alongside the instructive HTML landing page.
import { copyFile, readdir } from 'node:fs/promises';
const source = new URL('../docs/', import.meta.url);
for (const name of await readdir(source)) {
  if (name.endsWith('.md')) await copyFile(new URL(name, source), new URL(`dist/docs/${name}`, import.meta.url));
}
