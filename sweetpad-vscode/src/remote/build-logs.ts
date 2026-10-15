import type { RemoteClient } from "./extension";

/** Execute on the selected Mac; return the full text locally, not a Mac path. */
export async function extractRemoteBuildLog(client: RemoteClient, root: string): Promise<string> {
  const response = await client.execute(["--json", "build", "logs"], root);
  let result: { ok?: boolean; data?: { text?: string }; error?: { message?: string } };
  try {
    result = JSON.parse(response.stdout.trim() || response.stderr.trim().split("\n").at(-1)!);
  } catch {
    throw new Error(response.stderr.trim() || "SweetPad returned an invalid build log response.");
  }
  if (response.code !== 0 || !result.ok || typeof result.data?.text !== "string") {
    throw new Error(result.error?.message || response.stderr.trim() || "Could not extract the remote build log.");
  }
  return result.data.text;
}
