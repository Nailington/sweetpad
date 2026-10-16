import { createConnection } from "node:net";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as vscode from "vscode";

const ui = vi.hoisted(() => ({ password: vi.fn(), confirm: vi.fn(), error: vi.fn() }));
vi.mock("vscode", () => ({
  window: { showInputBox: ui.password, showWarningMessage: ui.confirm, showErrorMessage: ui.error },
}));

import { keychainKey, passwordKey, RemoteAuth } from "./auth";

describe("remote SSH authentication", () => {
  let auth: RemoteAuth;
  let stored: Map<string, string>;
  let secrets: vscode.SecretStorage;

  beforeEach(() => {
    vi.clearAllMocks();
    stored = new Map();
    secrets = {
      get: vi.fn(async (key: string) => stored.get(key)),
      store: vi.fn(async (key: string, value: string) => {
        stored.set(key, value);
      }),
      delete: vi.fn(async (key: string) => {
        stored.delete(key);
      }),
      onDidChange: vi.fn(),
    };
    auth = new RemoteAuth(secrets);
  });
  afterEach(() => auth.dispose());

  async function request(overrides: Record<string, unknown> = {}): Promise<string | null> {
    const env = await auth.environment();
    const port = Number(env.SWEETPAD_ASKPASS_ADDR!.split(":")[1]);
    return new Promise((resolve, reject) => {
      const socket = createConnection({ host: "127.0.0.1", port }, () => {
        socket.write(
          `${JSON.stringify({
            token: env.SWEETPAD_ASKPASS_TOKEN,
            host: "dev@mac.local",
            port: "",
            connection: "1-1",
            prompt: "dev@mac.local's password: ",
            confirm: false,
            ...overrides,
          })}\n`,
        );
      });
      let response = "";
      socket.setEncoding("utf8");
      socket.on("data", (data) => {
        response += data;
      });
      socket.on("error", reject);
      socket.on("end", () => {
        try {
          resolve(JSON.parse(response).answer);
        } catch (error) {
          reject(error);
        }
      });
    });
  }

  it("masks and stores passwords, then reuses them for later SSH connections", async () => {
    ui.password.mockResolvedValue("p@ss ' $ & ü");
    expect(await request()).toBe("p@ss ' $ & ü");
    expect(ui.password).toHaveBeenCalledWith(expect.objectContaining({ password: true, ignoreFocusOut: true }));
    expect(stored.get(passwordKey("dev@mac.local", null))).toBe("p@ss ' $ & ü");
    expect(await request({ connection: "1-2" })).toBe("p@ss ' $ & ü");
    expect(ui.password).toHaveBeenCalledOnce();
    expect(JSON.stringify(await auth.environment())).not.toContain("p@ss");
  });

  it("re-prompts when SSH rejects a stored password", async () => {
    stored.set(passwordKey("dev@mac.local", null), "old");
    ui.password.mockResolvedValue("corrected");
    expect(await request()).toBe("old");
    expect(await request()).toBe("corrected");
    expect(stored.get(passwordKey("dev@mac.local", null))).toBe("corrected");
  });

  it("deduplicates simultaneous prompts from sidebar queries", async () => {
    let finish!: (value: string) => void;
    ui.password.mockReturnValue(
      new Promise<string>((resolve) => {
        finish = resolve;
      }),
    );
    const first = request();
    const second = request({ connection: "1-2" });
    await vi.waitFor(() => expect(ui.password).toHaveBeenCalledOnce());
    finish("shared");
    expect(await Promise.all([first, second])).toEqual(["shared", "shared"]);
    expect(ui.password).toHaveBeenCalledOnce();
  });

  it("cancels without repeated prompts and removes a rejected saved password", async () => {
    stored.set(passwordKey("dev@mac.local", null), "wrong");
    ui.password.mockResolvedValue(undefined);
    expect(await request()).toBe("wrong");
    expect(await request()).toBeNull();
    expect(await request()).toBeNull();
    expect(stored.size).toBe(0);
    expect(ui.password).toHaveBeenCalledOnce();
  });

  it("requires explicit host trust and never sends the account password as confirmation", async () => {
    stored.set(passwordKey("dev@mac.local", null), "secret");
    ui.confirm.mockResolvedValueOnce(undefined).mockResolvedValueOnce("Connect");
    const prompt = { confirm: true, prompt: "Host key fingerprint. Continue connecting (yes/no)?" };
    expect(await request(prompt)).toBeNull();
    expect(await request(prompt)).toBe("yes");
    expect(ui.password).not.toHaveBeenCalled();
    expect(secrets.get).not.toHaveBeenCalled();
  });

  it("rejects requests with invalid tokens before reading secrets or prompting", async () => {
    expect(await request({ token: "0".repeat(64) })).toBeNull();
    expect(await request({ token: "ü".repeat(64) })).toBeNull();
    expect(secrets.get).not.toHaveBeenCalled();
    expect(ui.password).not.toHaveBeenCalled();
    expect(ui.error).not.toHaveBeenCalled();
  });

  it("keeps ports separate and forgets only the chosen endpoint", async () => {
    stored.set(passwordKey("dev@mac.local", null), "default");
    stored.set(passwordKey("dev@mac.local", 2222), "other");
    expect(await request({ port: "2222" })).toBe("other");
    await auth.forget("dev@mac.local", 2222);
    expect(stored.get(passwordKey("dev@mac.local", null))).toBe("default");
    expect(stored.has(passwordKey("dev@mac.local", 2222))).toBe(false);
  });

  it("does not cache key passphrases or verification codes as account passwords", async () => {
    ui.password.mockResolvedValue("one time secret");
    expect(await request({ prompt: "Enter passphrase for key '/tmp/key': " })).toBe("one time secret");
    expect(await request({ prompt: "Verification code: " })).toBe("one time secret");
    expect(secrets.store).not.toHaveBeenCalled();
    expect(secrets.get).not.toHaveBeenCalled();
  });

  it("defaults automatic keychain unlock to the saved SSH password without prompting", async () => {
    stored.set(passwordKey("dev@mac.local", null), "login secret");
    expect(await request({ kind: "keychain", savedOnly: true })).toBe("login secret");
    expect(ui.password).not.toHaveBeenCalled();
    expect(stored.has(keychainKey("dev@mac.local", null))).toBe(false);
  });

  it("prefers a separate keychain password and restores the SSH default when forgotten", async () => {
    stored.set(passwordKey("dev@mac.local", null), "login secret");
    stored.set(keychainKey("dev@mac.local", null), "keychain secret");
    expect(await request({ kind: "keychain", savedOnly: true })).toBe("keychain secret");
    await auth.forgetKeychain("dev@mac.local", null);
    expect(await request({ kind: "keychain", savedOnly: true })).toBe("login secret");
    expect(ui.password).not.toHaveBeenCalled();
  });

  it("does not ask for a keychain password when none is saved", async () => {
    expect(await request({ kind: "keychain", savedOnly: true })).toBeNull();
    expect(ui.password).not.toHaveBeenCalled();
  });

  it("saves a separate keychain password without changing the SSH credential", async () => {
    stored.set(passwordKey("dev@mac.local", null), "login");
    ui.password.mockResolvedValueOnce("separate");
    await auth.save("dev@mac.local", null, true);
    expect(stored.get(keychainKey("dev@mac.local", null))).toBe("separate");
    expect(stored.get(passwordKey("dev@mac.local", null))).toBe("login");
    ui.password.mockResolvedValueOnce("");
    await auth.save("dev@mac.local", null, true);
    expect(stored.has(keychainKey("dev@mac.local", null))).toBe(false);
  });
});
