function stripFrontmatter(source: string): string {
  return source.replace(/^---\n[\s\S]*?\n---\n*/, "");
}

function stripImportsAndExports(source: string): string {
  return source
    .replace(/^\s*import\s.+?;\s*$/gm, "")
    .replace(/^\s*export\s+(?:default\s+)?/gm, "");
}

function stripJsx(source: string): string {
  return source
    .replace(/<\/?[A-Z][^>\n]*\/?>/g, "")
    .replace(/<\/?[a-z][^>\n]*\/?>/g, "");
}

export function mdxToCleanMarkdown(source: string): string {
  return stripJsx(stripImportsAndExports(stripFrontmatter(source)))
    .replace(/\{`([^`]+)`\}/g, "`$1`")
    .replace(/\{"([^"]+)"\}/g, "$1")
    .replace(/\{'([^']+)'\}/g, "$1")
    .replace(/\{[^{}\n]+\}/g, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
