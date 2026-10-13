// Ship the repository's Markdown alongside the instructive HTML landing page (run after `next build`).
import { copyFile, mkdir, readdir } from 'node:fs/promises';
async function publish(source, destination) {
  await mkdir(destination, { recursive: true });
  for (const entry of await readdir(source, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      await publish(new URL(`${entry.name}/`, source), new URL(`${entry.name}/`, destination));
    } else if (entry.isFile() && entry.name.endsWith('.md')) {
      await copyFile(new URL(entry.name, source), new URL(entry.name, destination));
    }
  }
}
await publish(new URL('../docs/', import.meta.url), new URL('out/docs/', import.meta.url));
