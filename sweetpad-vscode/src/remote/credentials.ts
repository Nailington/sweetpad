import { spawn } from "node:child_process";

import type * as vscode from "vscode";

export type CredentialStorage = Pick<vscode.SecretStorage, "get" | "store" | "delete">;

/** The CLI owns the OS credential store. Its private helper uses pipes, so
 * passwords never become shell arguments or normal CLI JSON reports. */
export class RemoteCredentials implements CredentialStorage {
  constructor(
    private readonly legacy: CredentialStorage,
    private readonly executable: () => string,
  ) {}

  private request(
    action: "get" | "store" | "delete",
    key: string,
    value?: string,
  ): Promise<{ value: string | null; known: boolean }> {
    return new Promise((resolve, reject) => {
      const child = spawn(this.executable(), ["__credential-store"], {
        windowsHide: true,
        stdio: ["pipe", "pipe", "pipe"],
      });
      let output = "";
      const fail = () =>
        reject(
          new Error(
            "SweetPad cannot access the shared OS credential store. Update the local CLI; on Linux, start a Secret Service such as GNOME Keyring or KWallet.",
          ),
        );
      const timer = setTimeout(() => {
        child.kill();
        fail();
      }, 300_000);
      child.stdout.setEncoding("utf8");
      child.stdout.on("data", (data: string) => {
        output += data;
        if (output.length > 64 * 1024) {
          child.kill();
          fail();
        }
      });
      // Drain stderr, but do not attach child-process details or secret-bearing
      // protocol data to errors shown by VS Code or sent to telemetry.
      child.stderr.resume();
      child.stdin.on("error", fail);
      child.on("error", () => {
        clearTimeout(timer);
        fail();
      });
      child.on("close", (code) => {
        clearTimeout(timer);
        if (code !== 0) return fail();
        try {
          const reply = JSON.parse(output) as { value: string | null; known: boolean };
          if (reply.value !== null && typeof reply.value !== "string") return fail();
          if (typeof reply.known !== "boolean") return fail();
          resolve(reply);
        } catch {
          fail();
        }
      });
      child.stdin.end(`${JSON.stringify({ action, key, value })}\n`);
    });
  }

  async get(key: string): Promise<string | undefined> {
    const shared = await this.request("get", key);
    if (shared.known) {
      await this.legacy.delete(key);
      return shared.value ?? undefined;
    }
    const old = await this.legacy.get(key);
    if (old === undefined) return;
    // Delete the old entry only after its durable shared-store write succeeds.
    await this.store(key, old);
    return old;
  }

  async store(key: string, value: string): Promise<void> {
    await this.request("store", key, value);
    await this.legacy.delete(key);
  }

  async delete(key: string): Promise<void> {
    await this.request("delete", key);
    await this.legacy.delete(key);
  }
}
