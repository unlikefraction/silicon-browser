"use server";

export type EnvStatus = {
  sandbox: {
    hasSnapshot: boolean;
  };
};

export async function getEnvStatus(): Promise<EnvStatus> {
  return {
    sandbox: {
      hasSnapshot: !!process.env.SILICON_BROWSER_SNAPSHOT_ID,
    },
  };
}
