import { readFileSync } from "node:fs";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as vscode from "vscode";

const state = vi.hoisted(() => ({
  commands: new Map<string, (...args: any[]) => Promise<void>>(),
  providers: new Map<string, any>(),
  configChange: undefined as ((event: { affectsConfiguration(key: string): boolean }) => void) | undefined,
}));
vi.mock("vscode", () => ({
  TreeItem: class {
    constructor(
      public label: string,
      public collapsibleState = 0,
    ) {}
  },
  TreeItemCollapsibleState: { None: 0 },
  ThemeIcon: class {
    constructor(public id: string) {}
  },
  EventEmitter: class {
    private listeners = new Set<(value: unknown) => void>();
    event = (callback: (value: unknown) => void) => {
      this.listeners.add(callback);
      return { dispose: () => this.listeners.delete(callback) };
    };
    fire(value: unknown) {
      for (const callback of this.listeners) callback(value);
    }
    dispose() {
      this.listeners.clear();
    }
  },
  StatusBarAlignment: { Left: 1 },
  workspace: {
    getConfiguration: () => ({ get: (key: string) => (key === "remote.mac" ? "Studio Mac" : undefined) }),
    onDidChangeConfiguration: (callback: typeof state.configChange) => {
      state.configChange = callback;
      return { dispose() {} };
    },
  },
  commands: {
    executeCommand: vi.fn(),
    registerCommand: (id: string, callback: (...args: any[]) => Promise<void>) => {
      state.commands.set(id, callback);
      return { dispose() {} };
    },
  },
  window: {
    onDidCloseTerminal: () => ({ dispose: vi.fn() }),
    createStatusBarItem: () => ({ show() {}, dispose() {} }),
    registerTreeDataProvider: (id: string, provider: unknown) => {
      state.providers.set(id, provider);
      return { dispose() {} };
    },
    showErrorMessage: vi.fn(),
  },
}));

vi.mock("./features.js", () => ({ registerRemoteFeatures: () => ({}) }));

import { RemoteAuth } from "./auth";
import { activateRemote, RemoteClient } from "./extension";
import { RemoteMacItem, RemoteMacsTree } from "./mac-tree";

const pkg = JSON.parse(readFileSync(`${__dirname}/../../package.json`, "utf8"));
const saved = [
  { name: "Studio Mac", host: "dev@studio.local", port: null },
  { name: "Build Mac", host: "build@mac.local", port: 2222 },
];

describe("Macs sidebar", () => {
  let subscriptions: vscode.Disposable[];
  let tree: RemoteMacsTree;
  beforeEach(async () => {
    state.commands.clear();
    state.providers.clear();
    subscriptions = [];
    vi.spyOn(RemoteAuth.prototype, "environment").mockResolvedValue({});
    vi.spyOn(RemoteClient.prototype, "macs").mockResolvedValue(saved);
    await activateRemote({
      subscriptions,
      secrets: {},
      extension: { packageJSON: { contributes: { commands: [] } } },
    } as unknown as vscode.ExtensionContext);
    tree = state.providers.get("sweetpad.remote.macs.view");
  });
  afterEach(() => {
    for (const disposable of subscriptions) disposable.dispose();
    vi.restoreAllMocks();
  });

  it("lists every saved Mac and marks the selected one", async () => {
    const items = (await tree.getChildren()) as RemoteMacItem[];
    expect(items.map((item) => item.label)).toEqual(["Studio Mac", "Build Mac"]);
    expect(items[0].description).toBe("✓ · dev@studio.local");
    expect(items[1].description).toBe("build@mac.local");
    expect(items[1].tooltip).toContain("2222");
    expect(items[1].contextValue).toBe("remote-mac");
    expect(items[1].command?.arguments).toEqual([items[1]]);
    expect(await tree.getChildren(items[0])).toEqual([]);
  });

  it("routes row clicks and hover edits to that Mac", async () => {
    const item = (await tree.getChildren())[1];
    const select = vi.spyOn(RemoteClient.prototype, "selectMac").mockResolvedValue("Build Mac");
    const edit = vi.spyOn(RemoteClient.prototype, "editMac").mockResolvedValue();
    await state.commands.get("sweetpad.remote.selectMac")!(item);
    await state.commands.get("sweetpad.remote.editMac")!(item);
    expect(select).toHaveBeenCalledExactlyOnceWith("Build Mac");
    expect(edit).toHaveBeenCalledExactlyOnceWith("Build Mac");
  });

  it("refreshes the Macs list after adding, editing, and changing selection settings", async () => {
    const changed = vi.fn();
    tree.onDidChangeTreeData(changed);
    const add = vi.spyOn(RemoteClient.prototype, "addMac").mockResolvedValue();
    vi.spyOn(RemoteClient.prototype, "editMac").mockResolvedValue();
    await state.commands.get("sweetpad.remote.addMac")!();
    expect(add).toHaveBeenCalledOnce();
    await state.commands.get("sweetpad.remote.editMac")!();
    state.configChange!({ affectsConfiguration: (key) => key === "sweetpad.remote" });
    await state.commands.get("sweetpad.remote.refreshMacs")!();
    expect(changed).toHaveBeenCalledTimes(4);
  });

  it("provides an add button for the header and empty state, with inline edits only on saved rows", async () => {
    vi.mocked(RemoteClient.prototype.macs).mockResolvedValue([]);
    expect(await tree.getChildren()).toEqual([]);
    const macsView = pkg.contributes.views["speedpad-view"].find(
      (entry: any) => entry.id === "sweetpad.remote.macs.view",
    );
    expect(macsView).toMatchObject({ name: "Macs", when: "sweetpad.remoteMode", visibility: "visible" });
    const add = pkg.contributes.menus["view/title"].find((menu: any) => menu.command === "sweetpad.remote.addMac");
    expect(add.when).toContain("view == sweetpad.remote.macs.view");
    expect(add.group).toContain("navigation");
    const welcome = pkg.contributes.viewsWelcome.find((view: any) => view.view === "sweetpad.remote.macs.view");
    expect(welcome.contents).toContain("command:sweetpad.remote.addMac");
    const edit = pkg.contributes.menus["view/item/context"].find(
      (menu: any) => menu.command === "sweetpad.remote.editMac",
    );
    expect(edit.when).toContain("viewItem == remote-mac");
    expect(edit.when).toContain("view == sweetpad.remote.macs.view");
    expect(edit.group).toContain("inline");
  });

  it("shows registry errors with a retry action instead of an editable Mac", async () => {
    vi.mocked(RemoteClient.prototype.macs).mockRejectedValue(new Error("CLI unavailable"));
    const [item] = await tree.getChildren();
    expect(item.label).toBe("CLI unavailable");
    expect(item.command?.command).toBe("sweetpad.remote.refreshMacs");
    expect(item.contextValue).toBeUndefined();
  });
});
