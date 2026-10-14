import * as vscode from "vscode";

import type { Mac } from "./mac";

type MacClient = { readonly mac: string | undefined; macs(): Promise<Mac[]> };

export class RemoteMacItem extends vscode.TreeItem {
  constructor(
    readonly mac: Mac,
    selected: boolean,
  ) {
    super(mac.name, vscode.TreeItemCollapsibleState.None);
    this.id = mac.name;
    this.contextValue = "remote-mac";
    this.description = `${selected ? "✓ · " : ""}${mac.host}`;
    this.tooltip = `${mac.host}${mac.port ? ` · port ${mac.port}` : ""}${selected ? " · Selected" : ""}`;
    this.iconPath = new vscode.ThemeIcon("device-desktop");
    this.command = {
      command: "sweetpad.remote.selectMac",
      title: "Select Remote Mac",
      arguments: [this],
    };
  }
}

export class RemoteMacsTree implements vscode.TreeDataProvider<vscode.TreeItem>, vscode.Disposable {
  private readonly changed = new vscode.EventEmitter<vscode.TreeItem | undefined>();
  readonly onDidChangeTreeData = this.changed.event;

  constructor(private readonly client: MacClient) {}

  getTreeItem(item: vscode.TreeItem): vscode.TreeItem {
    return item;
  }

  async getChildren(parent?: vscode.TreeItem): Promise<vscode.TreeItem[]> {
    if (parent) return [];
    try {
      const macs = await this.client.macs();
      return macs.map((mac) => new RemoteMacItem(mac, mac.name === this.client.mac));
    } catch (error) {
      const item = new vscode.TreeItem(error instanceof Error ? error.message : String(error));
      item.iconPath = new vscode.ThemeIcon("warning");
      item.command = { command: "sweetpad.remote.refreshMacs", title: "Refresh Macs" };
      return [item];
    }
  }

  refresh(): void {
    this.changed.fire(undefined);
  }

  dispose(): void {
    this.changed.dispose();
  }
}
