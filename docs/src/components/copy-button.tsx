"use client";

import { useEffect, useState } from "react";

export function CopyButton({ code }: { code: string }) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  return (
    <button
      type="button"
      className="absolute right-3 top-3 rounded border border-white/10 bg-black/40 px-2 py-1 text-[11px] text-white/80 transition hover:bg-black/60 hover:text-white"
      onClick={async () => {
        await navigator.clipboard.writeText(code);
        setCopied(true);
      }}
      aria-label={copied ? "Copied code" : "Copy code"}
    >
      {copied ? "Copied" : "Copy"}
    </button>
  );
}
