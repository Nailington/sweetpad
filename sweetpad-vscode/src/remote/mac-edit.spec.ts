import { beforeEach, afterEach, describe, expect, it, vi, type MockInstance } from "vitest";

const mocks = vi.hoisted(() => ({
  execFile: vi.fn(),
  terminal: { show: vi.fn(), sendText: vi.fn(), dispose: vi.fn() },
  createTerminal: vi.fn(),
  input: vi.fn(),
  pick: vi.fn(),
  info: vi.fn(),
  update: vi.fn(),
  selection: {} as { globalValue?: string; workspaceValue?: string },
}));
vi.mock("node:child_process", () => ({ execFile: mocks.execFile }));
vi.mock("./mac-tree", () => ({ RemoteMacsTree: vi.fn() }));
vi.mock("vscode", () => ({
  ConfigurationTarget: { Global: 1, Workspace: 2, WorkspaceFolder: 3 },
  workspace: {
    workspaceFolders: [{ uri: { fsPath: "/project" } }],
    getConfiguration: () => ({
      get: (key: string) =>
        ({ "remote.mac": "Test Mac", "remote.cliPath": "C:\\Program Files\\SweetPad\\sweetpad.exe" })[key],
      inspect: () => mocks.selection,
      update: mocks.update,
    }),
  },
  window: {
    createTerminal: mocks.createTerminal,
    onDidCloseTerminal: () => ({ dispose: vi.fn() }),
    showInputBox: mocks.input,
    showQuickPick: mocks.pick,
    showInformationMessage: mocks.info,
  },
}));

import type { RemoteAuth } from "./auth";
import { RemoteClient } from "./extension";
import type { Mac } from "./mac";

describe("edit remote Mac", () => {
  const original: Mac = {
    name: "Test Mac",
    host: "dev@mac.local",
    port: 2222,
    identityFile: "/keys/original",
    auth: "ssh",
  };
  let client: RemoteClient;
  let query: MockInstance<RemoteClient["query"]>;
  let forget: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.input.mockReset();
    mocks.pick.mockReset();
    mocks.update.mockReset();
    mocks.selection = { workspaceValue: original.name };
    forget = vi.fn().mockResolvedValue(undefined);
    client = new RemoteClient({ forget } as unknown as RemoteAuth);
    vi.spyOn(client, "macs").mockResolvedValue([original]);
    query = vi.spyOn(client, "query").mockResolvedValue({});
    mocks.pick.mockResolvedValueOnce({ label: original.name, mac: original });
  });
  afterEach(() => vi.restoreAllMocks());

  function inputs(values: (string | undefined)[], method = "SSH config / private key") {
    for (const value of values) mocks.input.mockResolvedValueOnce(value);
    mocks.pick.mockResolvedValueOnce(method);
  }

  it("keeps the add flow creating a new selected Mac", async () => {
    mocks.pick.mockReset();
    inputs(["New Mac", "new@mac.local", ""], "Password");
    await client.addMac();
    expect(query).toHaveBeenCalledExactlyOnceWith(["remote", "add", "New Mac", "new@mac.local", "--auth", "password"]);
    expect(mocks.update).toHaveBeenCalledExactlyOnceWith("remote.mac", "New Mac", 2);
  });

  it("edits the hovered Mac directly instead of opening a Mac picker", async () => {
    const hovered = { ...original, name: "Other Mac", host: "other@mac.local" };
    vi.mocked(client.macs).mockResolvedValue([original, hovered]);
    mocks.pick.mockReset();
    inputs([hovered.name, hovered.host, "", "2222"]);
    await client.editMac(hovered.name);
    expect(mocks.pick).toHaveBeenCalledOnce();
    expect(mocks.pick.mock.calls[0][1].placeHolder).toContain("authenticate");
    expect(query.mock.calls[0][0]).toEqual([
      "remote",
      "add",
      hovered.name,
      hovered.host,
      "--replace",
      "--port",
      "2222",
    ]);
    expect(mocks.update).not.toHaveBeenCalled();
  });

  it("selects the clicked Mac directly", async () => {
    const clicked = { ...original, name: "Other Mac" };
    vi.mocked(client.macs).mockResolvedValue([original, clicked]);
    expect(await client.selectMac(clicked.name)).toBe(clicked.name);
    expect(mocks.pick).not.toHaveBeenCalled();
    expect(mocks.update).toHaveBeenCalledExactlyOnceWith("remote.mac", clicked.name, 2);
  });

  it("rejects a stale row without editing a different Mac", async () => {
    await expect(client.editMac("Removed Mac")).rejects.toThrow("no longer saved");
    await expect(client.selectMac("Removed Mac")).rejects.toThrow("no longer saved");
    expect(mocks.pick).not.toHaveBeenCalled();
    expect(query).not.toHaveBeenCalled();
    expect(mocks.update).not.toHaveBeenCalled();
  });

  it("prefills saved settings and replaces them without changing the active selection", async () => {
    inputs([original.name, original.host, "/keys/new key", "2200"]);
    await client.editMac();
    expect(mocks.input.mock.calls.map(([options]) => options.value)).toEqual([
      original.name,
      original.host,
      original.identityFile,
      "2222",
    ]);
    expect(query).toHaveBeenCalledExactlyOnceWith([
      "remote",
      "add",
      original.name,
      original.host,
      "--replace",
      "--identity-file",
      "/keys/new key",
      "--port",
      "2200",
    ]);
    expect(mocks.update).not.toHaveBeenCalled();
    expect(forget).toHaveBeenCalledWith(original.host, original.port);
  });

  it("switches to password authentication and clears a key and port", async () => {
    inputs([original.name, original.host, ""], "Password");
    await client.editMac();
    expect(query).toHaveBeenCalledExactlyOnceWith([
      "remote",
      "add",
      original.name,
      original.host,
      "--replace",
      "--auth",
      "password",
    ]);
    expect(mocks.input).toHaveBeenCalledTimes(3);
  });

  it("keeps password authentication selected first and preserves credentials on a rename", async () => {
    const passwordMac = { ...original, auth: "password" as const, identityFile: null };
    vi.mocked(client.macs).mockResolvedValue([passwordMac]);
    mocks.pick.mockReset().mockResolvedValueOnce({ label: original.name, mac: passwordMac });
    inputs(["Renamed Mac", original.host, "2222"], "Password");
    mocks.selection = { globalValue: original.name, workspaceValue: original.name };
    await client.editMac();
    expect(mocks.pick.mock.calls[1][0][0]).toBe("Password");
    expect(query.mock.calls).toEqual([
      [["remote", "add", "Renamed Mac", original.host, "--auth", "password", "--port", "2222"]],
      [["remote", "remove", original.name]],
    ]);
    expect(mocks.update.mock.calls).toEqual([
      ["remote.mac", "Renamed Mac", 1],
      ["remote.mac", "Renamed Mac", 2],
    ]);
    expect(forget).not.toHaveBeenCalled();
  });

  it("does not replace another saved Mac when renaming", async () => {
    vi.mocked(client.macs).mockResolvedValue([original, { ...original, name: "Taken" }]);
    inputs(["Taken", original.host, "", ""]);
    await expect(client.editMac()).rejects.toThrow("already exists");
    expect(query).not.toHaveBeenCalled();
    expect(mocks.update).not.toHaveBeenCalled();
  });

  it("does not mutate saved Macs or credentials when cancelled at the final prompt", async () => {
    inputs(["New name", "new@mac.local", "/keys/new", undefined]);
    await client.editMac();
    expect(query).not.toHaveBeenCalled();
    expect(mocks.update).not.toHaveBeenCalled();
    expect(forget).not.toHaveBeenCalled();
  });

  it("preserves the original entry and selection when saving the edit fails", async () => {
    inputs(["New name", "new@mac.local", "", ""]);
    query.mockRejectedValueOnce(new Error("Cannot save"));
    await expect(client.editMac()).rejects.toThrow("Cannot save");
    expect(query).toHaveBeenCalledOnce();
    expect(mocks.update).not.toHaveBeenCalled();
    expect(forget).not.toHaveBeenCalled();
  });

  it("keeps the original entry if updating the selected name fails", async () => {
    inputs(["New name", original.host, "", "2222"]);
    mocks.update.mockRejectedValueOnce(new Error("Settings are read-only"));
    await expect(client.editMac()).rejects.toThrow("Settings are read-only");
    expect(query).toHaveBeenCalledOnce();
    expect(forget).not.toHaveBeenCalled();
  });

  it("preserves a password still used by another saved Mac", async () => {
    vi.mocked(client.macs).mockResolvedValue([original, { ...original, name: "Shared endpoint" }]);
    inputs([original.name, "other@mac.local", "", ""]);
    await client.editMac();
    expect(forget).not.toHaveBeenCalled();
  });

  it("rejects invalid ports before saving", async () => {
    inputs([original.name, original.host, "", "70000"]);
    await expect(client.editMac()).rejects.toThrow("SSH port");
    expect(query).not.toHaveBeenCalled();
  });

  it("explains how to add a Mac when none are saved", async () => {
    vi.mocked(client.macs).mockResolvedValue([]);
    await client.editMac();
    expect(mocks.info).toHaveBeenCalledWith(expect.stringContaining("Add Remote Mac"));
    expect(mocks.pick).not.toHaveBeenCalled();
    expect(query).not.toHaveBeenCalled();
  });
});
