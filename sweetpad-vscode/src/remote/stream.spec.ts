import { EventEmitter } from "node:events";

import { beforeEach, describe, expect, it, vi } from "vitest";

import type { RemoteClient } from "./extension";

const mocks = vi.hoisted(() => ({ spawn: vi.fn(), stop: vi.fn(), panel: vi.fn() }));
vi.mock("vscode", () => ({
  ViewColumn: { Beside: 2 },
  Uri: { parse: (value: string) => ({ toString: () => value }) },
  env: { asExternalUri: async (value: unknown) => value },
  window: { createWebviewPanel: mocks.panel },
}));
vi.mock("./extension", () => ({ config: () => ({ get: () => undefined }) }));
vi.mock("node:net", () => ({
  createServer: () => ({
    once: () => {},
    listen: (_port: number, _host: string, callback: () => void) => queueMicrotask(callback),
    address: () => ({ port: 49100 }),
    close: (callback: () => void) => callback(),
  }),
}));
vi.mock("node:http", () => ({
  get: (_url: string, callback: (response: unknown) => void) => {
    queueMicrotask(() => callback({ statusCode: 200, resume: () => {} }));
    return { setTimeout: () => {}, on: () => {}, destroy: () => {} };
  },
}));
import { RemoteStreams } from "./stream";

function child() {
  return Object.assign(new EventEmitter(), {
    stdout: new EventEmitter(),
    stderr: new EventEmitter(),
  });
}
function client() {
  return { spawnBridge: mocks.spawn, stopBridge: mocks.stop } as unknown as RemoteClient;
}
describe("remote simulator stream lifecycle", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.spawn.mockImplementation(child);
  });
  it("waits for one shared startup for simultaneous callers", async () => {
    const streams = new RemoteStreams(client());
    const [first, second] = await Promise.all([streams.start("test-device"), streams.start("test-device")]);
    expect(first).toBe(second);
    expect(mocks.spawn).toHaveBeenCalledOnce();
    streams.dispose();
    expect(mocks.stop).toHaveBeenCalledWith(first.child);
  });
  it("an old helper exiting does not remove its replacement", async () => {
    const streams = new RemoteStreams(client());
    const first = await streams.start("test-device");
    streams.stop("test-device");
    const replacement = await streams.start("test-device");
    first.child.emit("exit", 0);
    expect(await streams.start("test-device")).toBe(replacement);
    expect(mocks.spawn).toHaveBeenCalledTimes(2);
    streams.dispose();
  });
  it("disposal during port allocation prevents a helper from starting", async () => {
    const streams = new RemoteStreams(client());
    const pending = streams.start("test-device");
    streams.dispose();
    await expect(pending).rejects.toThrow("disposed");
    expect(mocks.spawn).not.toHaveBeenCalled();
  });
});
