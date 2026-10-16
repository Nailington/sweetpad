import { EventEmitter } from "node:events";
import { PassThrough, Writable } from "node:stream";

import { beforeEach, describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({
  shared: new Map<string, string>(),
  forgotten: new Set<string>(),
  calls: [] as unknown[][],
  requests: [] as { action: string; key: string; value?: string }[],
  fail: false,
}));
vi.mock("node:child_process", () => ({
  spawn: (...args: unknown[]) => {
    state.calls.push(args);
    const child = Object.assign(new EventEmitter(), {
      stdout: new PassThrough(),
      stderr: new PassThrough(),
      stdin: null as unknown as Writable,
      kill: vi.fn(),
    });
    child.stdin = new Writable({
      write(data, _encoding, callback) {
        const request = JSON.parse(data.toString());
        state.requests.push(request);
        queueMicrotask(() => {
          if (!state.fail) {
            if (request.action === "store") state.shared.set(request.key, request.value);
            if (request.action === "delete") state.shared.delete(request.key);
            if (request.action === "delete") state.forgotten.add(request.key);
            if (request.action === "store") state.forgotten.delete(request.key);
            child.stdout.end(
              JSON.stringify({
                value: request.action === "get" ? (state.shared.get(request.key) ?? null) : null,
                known: state.shared.has(request.key) || state.forgotten.has(request.key),
              }),
            );
          }
          child.emit("close", state.fail ? 1 : 0);
        });
        callback();
      },
    });
    return child;
  },
}));

import { RemoteCredentials } from "./credentials";

describe("shared CLI credential storage", () => {
  const key = 'sweetpad.remote.password:["dev@mac",""]';
  let legacy: Map<string, string>;
  let credentials: RemoteCredentials;
  beforeEach(() => {
    state.shared.clear();
    state.forgotten.clear();
    state.calls.length = 0;
    state.requests.length = 0;
    state.fail = false;
    legacy = new Map();
    credentials = new RemoteCredentials(
      {
        get: async (key) => legacy.get(key),
        store: async (key, value) => {
          legacy.set(key, value);
        },
        delete: async (key) => {
          legacy.delete(key);
        },
      },
      () => "/path with spaces/sweetpad",
    );
  });
  it("uses the same saved credential as standalone CLI commands", async () => {
    state.shared.set(key, "cli password");
    expect(await credentials.get(key)).toBe("cli password");
  });
  it("migrates old editor passwords only after the shared write succeeds", async () => {
    legacy.set(key, "old editor password");
    expect(await credentials.get(key)).toBe("old editor password");
    expect(state.shared.get(key)).toBe("old editor password");
    expect(legacy.has(key)).toBe(false);
    expect(state.calls.every((call) => JSON.stringify(call).includes("__credential-store"))).toBe(true);
    expect(JSON.stringify(state.calls)).not.toContain("old editor password");
  });
  it("keeps legacy credentials when the OS store is unavailable", async () => {
    legacy.set(key, "private value");
    state.fail = true;
    await expect(credentials.get(key)).rejects.toThrow("OS credential store");
    expect(legacy.get(key)).toBe("private value");
  });
  it("writes through stdin and forgets credentials from both stores", async () => {
    await credentials.store(key, "p@ss '$ ü");
    expect(state.shared.get(key)).toBe("p@ss '$ ü");
    expect(JSON.stringify(state.calls)).not.toContain("p@ss");
    await credentials.delete(key);
    expect(state.shared.has(key)).toBe(false);
    expect(await credentials.get(key)).toBeUndefined();
  });
  it("does not resurrect an old editor password after it was forgotten in the CLI", async () => {
    legacy.set(key, "old editor password");
    state.forgotten.add(key);
    expect(await credentials.get(key)).toBeUndefined();
    expect(legacy.has(key)).toBe(false);
    expect(state.shared.has(key)).toBe(false);
  });
});
