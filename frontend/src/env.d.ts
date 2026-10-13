declare namespace NodeJS {
  interface ProcessEnv {
    /** The validated backend origin, inlined at build time by next.config.mjs. */
    readonly SB_BACKEND_ORIGIN: string;
  }
}
