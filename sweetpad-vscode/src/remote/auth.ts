import { randomBytes, timingSafeEqual } from "node:crypto";
import { createServer, type Server, type Socket } from "node:net";

import * as vscode from "vscode";

import type { CredentialStorage } from "./credentials.js";

type Request = {
  token: string;
  host: string;
  port: string;
  connection: string;
  prompt: string;
  confirm: boolean;
  kind?: "keychain";
  savedOnly?: boolean;
};

export function passwordKey(host: string, port: string | number | null): string {
  return `sweetpad.remote.password:${JSON.stringify([host, String(port ?? "")])}`;
}

export function keychainKey(host: string, port: string | number | null): string {
  return `sweetpad.remote.keychain:${JSON.stringify([host, String(port ?? "")])}`;
}

/** Local askpass broker shared by background CLI calls and build terminals. */
export class RemoteAuth implements vscode.Disposable {
  private readonly token = randomBytes(32).toString("hex");
  private server?: Server;
  private starting?: Promise<NodeJS.ProcessEnv>;
  private readonly sockets = new Set<Socket>();
  private readonly attempts = new Map<string, { used: boolean; cancelled: boolean; at: number }>();
  private readonly pending = new Map<string, Promise<string | undefined>>();
  private disposed = false;

  constructor(private readonly secrets: CredentialStorage) {}

  environment(): Promise<NodeJS.ProcessEnv> {
    return (this.starting ??= this.listen().catch((error) => {
      this.starting = undefined;
      throw error;
    }));
  }

  private async listen(): Promise<NodeJS.ProcessEnv> {
    if (this.disposed) throw new Error("SSH authentication has stopped.");
    const server = createServer((socket) => this.accept(socket));
    this.server = server;
    await new Promise<void>((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "127.0.0.1", () => {
        server.off("error", reject);
        resolve();
      });
    });
    const address = server.address();
    if (!address || typeof address === "string") throw new Error("Cannot start SSH authentication.");
    server.unref();
    return {
      SWEETPAD_ASKPASS_ADDR: `127.0.0.1:${address.port}`,
      SWEETPAD_ASKPASS_TOKEN: this.token,
    };
  }

  private accept(socket: Socket): void {
    this.sockets.add(socket);
    socket.on("close", () => this.sockets.delete(socket));
    socket.on("error", () => socket.destroy());
    socket.setTimeout(300_000, () => socket.destroy());
    let data = "";
    socket.setEncoding("utf8");
    const receive = (chunk: string) => {
      data += chunk;
      if (Buffer.byteLength(data) > 64 * 1024) return socket.destroy();
      if (!data.includes("\n")) return;
      socket.off("data", receive);
      void this.respond(data.slice(0, data.indexOf("\n")))
        .then((answer) => socket.end(`${JSON.stringify({ answer: answer ?? null })}\n`))
        .catch(() => {
          void vscode.window.showErrorMessage(
            "SweetPad could not access saved credentials. Check the local CLI and OS credential store.",
          );
          socket.end('{"answer":null}\n');
        });
    };
    socket.on("data", receive);
  }

  private async respond(line: string): Promise<string | undefined> {
    const request = JSON.parse(line) as Request;
    if (
      typeof request.token !== "string" ||
      Buffer.byteLength(request.token) !== Buffer.byteLength(this.token) ||
      !timingSafeEqual(Buffer.from(request.token), Buffer.from(this.token)) ||
      typeof request.host !== "string" ||
      typeof request.port !== "string" ||
      typeof request.connection !== "string" ||
      typeof request.prompt !== "string" ||
      typeof request.confirm !== "boolean"
    ) {
      return undefined;
    }
    // Host trust remains an explicit decision, never a stored password response.
    if (request.confirm || /yes\/no|fingerprint/i.test(request.prompt)) {
      const choice = await vscode.window.showWarningMessage(
        `SSH host verification for ${request.host}:\n${request.prompt}`,
        { modal: true },
        "Connect",
      );
      return choice === "Connect" ? "yes" : undefined;
    }
    if (request.kind === "keychain") {
      if (request.savedOnly !== true) return undefined;
      return (
        (await this.secrets.get(keychainKey(request.host, request.port))) ??
        (await this.secrets.get(passwordKey(request.host, request.port)))
      );
    }
    const key = passwordKey(request.host, request.port);
    const connectionKey = JSON.stringify([key, request.connection, request.prompt]);
    const now = Date.now();
    for (const [id, attempt] of this.attempts) {
      if (now - attempt.at > 300_000) this.attempts.delete(id);
    }
    const attempt = this.attempts.get(connectionKey) ?? { used: false, cancelled: false, at: now };
    this.attempts.set(connectionKey, attempt);
    if (attempt.cancelled || this.disposed) return undefined;
    // Cache account passwords only. Key passphrases and one-time codes always prompt.
    const accountPassword = /password\s*:\s*$/i.test(request.prompt) && !/passphrase|one.time/i.test(request.prompt);
    if (accountPassword && !attempt.used) {
      attempt.used = true;
      const stored = await this.secrets.get(key);
      if (stored !== undefined) return stored;
    }
    const promptKey = accountPassword ? key : connectionKey;
    let pending = this.pending.get(promptKey);
    if (!pending) {
      pending = this.prompt(request, key, accountPassword).finally(() => this.pending.delete(promptKey));
      this.pending.set(promptKey, pending);
    }
    const answer = await pending;
    if (answer === undefined) attempt.cancelled = true;
    return answer;
  }

  private async prompt(request: Request, key: string, remember: boolean): Promise<string | undefined> {
    const answer = await vscode.window.showInputBox({
      title: `SSH authentication · ${request.host}`,
      prompt: remember ? "Mac account password (saved in the OS credential store for CLI and editor)" : request.prompt,
      password: true,
      ignoreFocusOut: true,
      validateInput: (value) =>
        value.includes("\n") || value.includes("\r") || value.includes("\0") ? "Enter a single line." : undefined,
    });
    if (this.disposed) return undefined;
    if (remember) {
      if (answer === undefined) await this.secrets.delete(key);
      else await this.secrets.store(key, answer);
    }
    return answer;
  }

  async forget(host: string, port: number | null): Promise<void> {
    await this.secrets.delete(passwordKey(host, port));
  }

  async save(host: string, port: number | null, keychain: boolean): Promise<void> {
    const key = keychain ? keychainKey(host, port) : passwordKey(host, port);
    const answer = await vscode.window.showInputBox({
      title: `${keychain ? "Login keychain" : "SSH login"} password · ${host}`,
      prompt: keychain
        ? "Saved for CLI and editor. Leave blank to use the saved SSH password."
        : "Saved in the OS credential store for CLI and editor.",
      password: true,
      ignoreFocusOut: true,
      validateInput: (value) => (/[\r\n\0]/.test(value) ? "Enter a single line." : undefined),
    });
    if (answer === undefined) return;
    if (keychain && answer === "") await this.secrets.delete(key);
    else await this.secrets.store(key, answer);
  }

  async forgetKeychain(host: string, port: number | null): Promise<void> {
    await this.secrets.delete(keychainKey(host, port));
  }

  dispose(): void {
    this.disposed = true;
    for (const socket of this.sockets) socket.destroy();
    this.server?.close();
    this.attempts.clear();
    this.pending.clear();
  }
}
