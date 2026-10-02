import { existsSync } from "node:fs";

import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  execFile: vi.fn(),
  terminal: { show: vi.fn(), sendText: vi.fn(), dispose: vi.fn() },
  createTerminal: vi.fn(),
  closeTerminal: undefined as ((terminal: unknown) => void) | undefined,
  closeSubscription: { dispose: vi.fn() },
}));
vi.mock("node:child_process", () => ({ execFile: mocks.execFile }));
vi.mock("vscode", () => ({
  workspace: {
    getConfiguration: () => ({
      get: (key: string) =>
        ({
          "remote.mac": "Test Mac",
          "remote.cliPath": "C:\\Program Files\\SweetPad\\sweetpad.exe",
        })[key],
    }),
  },
  window: {
    createTerminal: mocks.createTerminal,
    onDidCloseTerminal: (handler: (terminal: unknown) => void) => {
      mocks.closeTerminal = handler;
      return mocks.closeSubscription;
    },
  },
}));

import { RemoteClient } from "./extension";

describe("remote GUI client", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.createTerminal.mockReturnValue(mocks.terminal);
  });
  afterEach(() => vi.restoreAllMocks());

  it("starts a native Windows executable with argument boundaries preserved", async () => {
    vi.spyOn(process, "platform", "get").mockReturnValue("win32");
    const client = new RemoteClient();
    await client.run(["build", "--scheme", "App & Tools"], "Build", "C:\\Project Folder");
    expect(mocks.createTerminal).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "Build · Test Mac",
        cwd: "C:\\Project Folder",
        shellPath: "C:\\Program Files\\SweetPad\\sweetpad.exe",
        shellArgs: ["--remote", "Test Mac", "build", "--scheme", "App & Tools"],
        env: expect.objectContaining({ SWEETPAD_CANCEL_FILE: expect.any(String) }),
      }),
    );
    expect(mocks.terminal.sendText).not.toHaveBeenCalled();
    client.stop();
    const cancelFile = mocks.createTerminal.mock.calls[0][0].env.SWEETPAD_CANCEL_FILE;
    expect(existsSync(cancelFile)).toBe(true);
    expect(mocks.terminal.dispose).not.toHaveBeenCalled();
    mocks.closeTerminal?.(mocks.terminal);
    expect(existsSync(cancelFile)).toBe(false);
  });

  it("does not interrupt a closed terminal and releases its listener", async () => {
    vi.spyOn(process, "platform", "get").mockReturnValue("win32");
    const client = new RemoteClient();
    await client.run(["build"], "Build");
    mocks.closeTerminal?.(mocks.terminal);
    client.dispose();
    expect(mocks.terminal.sendText).not.toHaveBeenCalled();
    expect(mocks.closeSubscription.dispose).toHaveBeenCalledOnce();
  });

  it("quotes Unix terminal arguments including apostrophes", async () => {
    vi.spyOn(process, "platform", "get").mockReturnValue("linux");
    await new RemoteClient().run(["build", "--scheme", "App's Tools"], "Build");
    expect(mocks.terminal.sendText.mock.calls[0][0]).toContain("'App'\\''s Tools'");
  });

  it("reads the CLI JSON envelope without a shell", async () => {
    mocks.execFile.mockImplementation((_file, _args, _options, callback) =>
      callback(null, '{"ok":true,"data":{"schemes":[]}}', ""),
    );
    expect(await new RemoteClient().query(["scheme", "list"], "/project")).toEqual({ schemes: [] });
    expect(mocks.execFile.mock.calls[0][1]).toEqual(["-o", "json", "--non-interactive", "scheme", "list"]);
  });

  it("rejects malformed CLI output", async () => {
    mocks.execFile.mockImplementation((_file, _args, _options, callback) => callback(null, "not JSON", ""));
    await expect(new RemoteClient().query(["devices"])).rejects.toThrow("invalid JSON");
  });

  it("shows the structured Mac error returned on stderr", async () => {
    mocks.execFile.mockImplementation((_file, _args, _options, callback) =>
      callback(new Error("exit 1"), "", 'SSH note\n{"ok":false,"error":{"message":"No scheme selected"}}\n'),
    );
    await expect(new RemoteClient().query(["scheme", "list"])).rejects.toThrow("No scheme selected");
  });
});
