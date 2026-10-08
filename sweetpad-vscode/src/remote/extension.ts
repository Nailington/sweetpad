import { execFile, spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import * as vscode from "vscode";

function execFileAsync(
  file: string,
  args: string[],
  options: { cwd?: string; maxBuffer: number },
): Promise<{ stdout: string }> {
  return new Promise((resolve, reject) => {
    execFile(file, args, options, (error, stdout, stderr) => {
      if (error) reject(Object.assign(error, { stdout, stderr }));
      else resolve({ stdout });
    });
  });
}

type Mac = { name: string; host: string; port: number | null };
type Envelope<T> = { ok: boolean; data?: T; error?: { message?: string } };
type Scheme = { name: string; selected: boolean };
type Destination = {
  kind: string;
  name: string;
  os: string;
  osVersion: string;
  udid: string | null;
  booted: boolean | null;
  destination: string;
};

export function config() {
  return vscode.workspace.getConfiguration("sweetpad");
}

function macSettingTarget(): vscode.ConfigurationTarget {
  return vscode.workspace.workspaceFolders?.length
    ? vscode.ConfigurationTarget.Workspace
    : vscode.ConfigurationTarget.Global;
}

export function projectRoot(): string {
  const active = vscode.window.activeTextEditor?.document.uri;
  const folder = active && vscode.workspace.getWorkspaceFolder(active);
  const root = folder ?? vscode.workspace.workspaceFolders?.[0];
  if (!root) throw new Error("Open a project folder to run SweetPad on a Mac.");
  return root.uri.fsPath;
}

export function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

export class RemoteClient {
  private terminals = new Set<vscode.Terminal>();
  private cancellationDirectories = new Map<vscode.Terminal, string>();
  private backgroundDirectories = new Map<ChildProcess, string>();
  private readonly closed = vscode.window.onDidCloseTerminal((terminal) => {
    this.terminals.delete(terminal);
    const directory = this.cancellationDirectories.get(terminal);
    if (directory) rmSync(directory, { recursive: true, force: true });
    this.cancellationDirectories.delete(terminal);
  });

  get cli(): string {
    return config().get<string>("remote.cliPath") || "sweetpad";
  }

  get mac(): string | undefined {
    return config().get<string>("remote.mac") || undefined;
  }

  private commandArgs(args: string[]): string[] {
    const result = [...args];
    const developerDir = config().get<string>("remote.developerDir");
    if (developerDir && !result.includes("--developer-dir")) result.unshift("--developer-dir", developerDir);
    const command = args.find((arg) =>
      [
        "bridge",
        "remote",
        "build",
        "run",
        "test",
        "clean",
        "app",
        "simulator",
        "devices",
        "scheme",
        "dependency",
        "doctor",
        "bsp",
        "format",
        "open",
      ].includes(arg),
    );
    const extra = config().get<string[]>("remote.xcodebuildArgs");
    if (command && ["build", "run", "test", "clean"].includes(command) && extra?.length && !result.includes("--")) {
      const next = args[args.indexOf(command) + 1];
      if (command !== "test" || !next || next.startsWith("-")) result.push("--", ...extra);
    }
    return result;
  }

  bridgeArgs(service: string, args: string[] = []): string[] {
    if (!this.mac) throw new Error("Select a remote Mac first.");
    const options = ["bridge", service, "--remote", this.mac];
    const developerDir = config().get<string>("remote.developerDir");
    if (developerDir) options.push("--developer-dir", developerDir);
    if (args.length) options.push("--", ...args);
    return options;
  }

  spawnBridge(service: string, args: string[] = [], cwd = projectRoot()) {
    const directory = mkdtempSync(path.join(tmpdir(), "sweetpad-service-"));
    const child = spawn(this.cli, this.bridgeArgs(service, args), {
      cwd,
      windowsHide: true,
      env: { ...process.env, SWEETPAD_CANCEL_FILE: path.join(directory, "cancel") },
    });
    this.backgroundDirectories.set(child, directory);
    child.once("close", () => {
      rmSync(directory, { recursive: true, force: true });
      this.backgroundDirectories.delete(child);
    });
    return child;
  }

  stopBridge(child: ChildProcess): void {
    const directory = this.backgroundDirectories.get(child);
    if (directory) writeFileSync(path.join(directory, "cancel"), "");
    child.stdin?.end();
  }

  async execute(args: string[], cwd = projectRoot(), token?: vscode.CancellationToken) {
    const mac = await this.ensureMac();
    if (!mac) throw new Error("No remote Mac selected.");
    const directory = mkdtempSync(path.join(tmpdir(), "sweetpad-cancel-"));
    const cancelFile = path.join(directory, "cancel");
    const child = spawn(this.cli, ["--non-interactive", "--remote", mac, ...this.commandArgs(args)], {
      cwd,
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, SWEETPAD_CANCEL_FILE: cancelFile },
    });
    this.backgroundDirectories.set(child, directory);
    let stdout = "",
      stderr = "",
      overflow = false;
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (data: string) => {
      if (overflow) return;
      stdout += data;
      if (stdout.length > 64 * 1024 * 1024) {
        overflow = true;
        cancel();
      }
    });
    child.stderr.on("data", (data: string) => {
      stderr = (stderr + data).slice(-16 * 1024 * 1024);
    });
    const cancel = () => writeFileSync(cancelFile, "");
    const subscription = token?.onCancellationRequested(cancel);
    if (token?.isCancellationRequested) cancel();
    try {
      const code = await new Promise<number>((resolve, reject) => {
        child.once("error", reject);
        child.once("close", (exitCode) => resolve(exitCode ?? 1));
      });
      if (overflow) throw new Error("Remote command output exceeded 64 MiB. Save large results to a file instead.");
      return { stdout, stderr, code };
    } finally {
      subscription?.dispose();
      this.backgroundDirectories.delete(child);
      rmSync(directory, { recursive: true, force: true });
    }
  }

  async query<T>(args: string[], cwd?: string): Promise<T> {
    let stdout: string;
    try {
      ({ stdout } = await execFileAsync(this.cli, ["-o", "json", "--non-interactive", ...this.commandArgs(args)], {
        cwd,
        maxBuffer: 16 * 1024 * 1024,
      }));
    } catch (error) {
      const failure = error as Error & { stderr?: string; stdout?: string };
      const candidates = [failure.stdout, ...(failure.stderr?.trim().split("\n").toReversed() ?? [])];
      for (const candidate of candidates) {
        if (!candidate) continue;
        let envelope: Envelope<T> | undefined;
        try {
          envelope = JSON.parse(candidate) as Envelope<T>;
        } catch {
          /* Fall back to the process error when stdout is not JSON. */
        }
        if (envelope?.error?.message) throw new Error(envelope.error.message, { cause: error });
      }
      throw new Error(failure.stderr?.trim() || failure.message, { cause: error });
    }
    let envelope: Envelope<T>;
    try {
      envelope = JSON.parse(stdout) as Envelope<T>;
    } catch {
      throw new Error(`SweetPad returned invalid JSON: ${stdout.slice(0, 200)}`);
    }
    if (!envelope.ok || envelope.data === undefined) {
      throw new Error(envelope.error?.message || "SweetPad command failed.");
    }
    return envelope.data;
  }

  async macs(): Promise<Mac[]> {
    const data = await this.query<{ macs: Mac[] }>(["remote", "list"]);
    return data.macs;
  }

  async selectMac(): Promise<string | undefined> {
    const macs = await this.macs();
    if (macs.length === 0) {
      void vscode.window.showInformationMessage("No Macs saved. Run SweetPad: Add Remote Mac first.");
      return undefined;
    }
    const picked = await vscode.window.showQuickPick(
      macs.map((mac) => ({ label: mac.name, description: mac.host })),
      { placeHolder: "Select the Mac used by SweetPad in this workspace" },
    );
    if (!picked) return undefined;
    await config().update("remote.mac", picked.label, macSettingTarget());
    return picked.label;
  }

  async ensureMac(): Promise<string | undefined> {
    return this.mac ?? (await this.selectMac());
  }

  async run(args: string[], name: string, cwd?: string): Promise<void> {
    const mac = await this.ensureMac();
    if (!mac) return;
    const cancellationDirectory = mkdtempSync(path.join(tmpdir(), "sweetpad-terminal-"));
    // VS Code's terminal host can outlive the extension host and retain its
    // original environment. Forward the CLI's registry and transport settings.
    const env: Record<string, string> = {
      SWEETPAD_CANCEL_FILE: path.join(cancellationDirectory, "cancel"),
    };
    for (const key of ["APPDATA", "LOCALAPPDATA", "SWEETPAD_REMOTE_EXECUTABLE", "SWEETPAD_REMOTE_ROOT"]) {
      const value = process.env[key];
      if (value !== undefined) env[key] = value;
    }
    const terminal =
      process.platform === "win32"
        ? vscode.window.createTerminal({
            name: `${name} · ${mac}`,
            cwd,
            shellPath: this.cli,
            shellArgs: ["--remote", mac, ...this.commandArgs(args)],
            env,
          })
        : vscode.window.createTerminal({ name: `${name} · ${mac}`, cwd, env });
    this.terminals.add(terminal);
    this.cancellationDirectories.set(terminal, cancellationDirectory);
    terminal.show();
    if (process.platform !== "win32") {
      terminal.sendText([this.cli, "--remote", mac, ...this.commandArgs(args)].map(shellQuote).join(" "), true);
    }
  }

  stop(): void {
    // Keep the terminal alive while the CLI forwards SIGINT and retrieves
    // artifacts. Disposing it terminates the client before that can finish.
    for (const terminal of this.terminals) {
      const directory = this.cancellationDirectories.get(terminal);
      if (terminal.exitStatus === undefined && directory) writeFileSync(path.join(directory, "cancel"), "");
    }
  }

  dispose(): void {
    this.stop();
    for (const child of this.backgroundDirectories.keys()) this.stopBridge(child);
    this.closed.dispose();
    this.terminals.clear();
  }

  async shutdown(): Promise<void> {
    this.stop();
    await Promise.all(
      [...this.backgroundDirectories.keys()].map(
        (child) =>
          new Promise<void>((resolve) => {
            if (child.exitCode !== null || child.signalCode !== null) {
              resolve();
              return;
            }
            const timer = setTimeout(() => {
              child.kill();
              resolve();
            }, 10000);
            child.once("close", () => {
              clearTimeout(timer);
              resolve();
            });
            this.stopBridge(child);
          }),
      ),
    );
  }
}

class RefreshableTree implements vscode.TreeDataProvider<vscode.TreeItem> {
  private changed = new vscode.EventEmitter<vscode.TreeItem | undefined>();
  readonly onDidChangeTreeData = this.changed.event;

  constructor(
    private readonly client: RemoteClient,
    private readonly kind: "scheme" | "destination",
  ) {}

  refresh(): void {
    this.changed.fire(undefined);
  }

  getTreeItem(item: vscode.TreeItem): vscode.TreeItem {
    return item;
  }

  async getChildren(): Promise<vscode.TreeItem[]> {
    const mac = this.client.mac;
    if (!mac) {
      const item = new vscode.TreeItem("Select a remote Mac");
      item.command = { command: "sweetpad.remote.selectMac", title: "Select Remote Mac" };
      return [item];
    }
    try {
      if (this.kind === "scheme") {
        const data = await this.client.query<{ schemes: Scheme[] }>(["scheme", "list", "--remote", mac], projectRoot());
        return data.schemes.map((scheme) => {
          const item = new vscode.TreeItem(scheme.name);
          (item as vscode.TreeItem & { scheme: string }).scheme = scheme.name;
          item.contextValue = "build-item&status=idle";
          item.description = config().get<string>("remote.scheme") === scheme.name || scheme.selected ? "✓" : "";
          item.iconPath = new vscode.ThemeIcon("package");
          return item;
        });
      }
      const data = await this.client.query<{ destinations: Destination[] }>(
        ["devices", "--remote", mac],
        projectRoot(),
      );
      return data.destinations.map((destination) => {
        const item = new vscode.TreeItem(destination.name);
        Object.assign(item, { destination: destination.destination, udid: destination.udid });
        const selected = config().get<string>("remote.destination") === destination.destination;
        item.description =
          `${selected ? "✓ · " : ""}${destination.os} ${destination.osVersion}${destination.booted ? " · booted" : ""}`.trim();
        item.tooltip = destination.destination;
        item.contextValue =
          destination.kind === "simulator"
            ? `destination-item-simulator&status=${destination.booted ? "booted" : "shutdown"}`
            : "destination-item-device";
        item.iconPath = new vscode.ThemeIcon(destination.kind === "simulator" ? "device-mobile" : "device-desktop");
        return item;
      });
    } catch (error) {
      const item = new vscode.TreeItem(error instanceof Error ? error.message : String(error));
      item.iconPath = new vscode.ThemeIcon("warning");
      return [item];
    }
  }

  dispose(): void {
    this.changed.dispose();
  }
}

type SchemeItem = vscode.TreeItem & { scheme?: string };
type DestinationItem = vscode.TreeItem & { destination?: string; udid?: string };

/** Register the Mac registry and ad-hoc command actions in either activation mode. */
export function registerMacCommands(
  register: (id: string, action: () => Promise<void>) => void,
  client: RemoteClient,
  refresh: () => Promise<void> | void,
): void {
  register("sweetpad.remote.selectMac", async () => {
    const mac = await client.selectMac();
    if (mac) await refresh();
  });
  register("sweetpad.remote.addMac", async () => {
    const name = await vscode.window.showInputBox({ prompt: "Name for this Mac" });
    if (!name) return;
    const host = await vscode.window.showInputBox({
      prompt: "SSH target (user@host or SSH config alias)",
    });
    if (!host) return;
    const key = await vscode.window.showInputBox({
      prompt: "Private key path (leave blank for your normal SSH config/agent)",
    });
    if (key === undefined) return;
    const port = await vscode.window.showInputBox({
      prompt: "SSH port (leave blank for SSH config/default)",
    });
    if (port === undefined) return;
    if (port && (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535)) {
      throw new Error("SSH port must be between 1 and 65535.");
    }
    const args = ["remote", "add", name, host];
    if (key) args.push("--identity-file", key);
    if (port) args.push("--port", port);
    await client.query(args);
    await config().update("remote.mac", name, macSettingTarget());
    await refresh();
  });
  register("sweetpad.remote.removeMac", async () => {
    const macs = await client.macs();
    const picked = await vscode.window.showQuickPick(
      macs.map((mac) => mac.name),
      { placeHolder: "Remove saved Mac" },
    );
    if (!picked) return;
    await client.query(["remote", "remove", picked]);
    if (client.mac === picked) {
      if (vscode.workspace.workspaceFolders?.length) {
        await config().update("remote.mac", undefined, vscode.ConfigurationTarget.Workspace);
      }
      await config().update("remote.mac", undefined, vscode.ConfigurationTarget.Global);
    }
    await refresh();
  });

  register("sweetpad.remote.command", async () => {
    const command = await vscode.window.showInputBox({
      prompt: "SweetPad CLI arguments (for example: simulator list)",
    });
    if (!command) return;
    const { parse } = await import("shell-quote");
    const args = parse(command);
    if (args.some((arg) => typeof arg !== "string")) throw new Error("Only command arguments are supported.");
    await client.run(args as string[], "SweetPad Remote", projectRoot());
  });
}

export async function activateRemote(context: vscode.ExtensionContext) {
  const client = new RemoteClient();
  const schemes = new RefreshableTree(client, "scheme");
  const destinations = new RefreshableTree(client, "destination");
  const status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 0);
  status.name = "SweetPad: Remote Mac";
  status.command = "sweetpad.remote.selectMac";
  const updateStatus = () => {
    status.text = `$(remote) ${client.mac ?? "Select Mac"}`;
  };
  const refresh = () => {
    updateStatus();
    schemes.refresh();
    destinations.refresh();
  };
  const registered = new Set<string>();
  const register = (id: string, action: (...args: any[]) => Promise<void> | void) => {
    registered.add(id);
    context.subscriptions.push(
      vscode.commands.registerCommand(id, async (...args: unknown[]) => {
        try {
          await action(...args);
        } catch (error) {
          void vscode.window.showErrorMessage(`SweetPad: ${error instanceof Error ? error.message : String(error)}`);
        }
      }),
    );
  };

  context.subscriptions.push(client, schemes, destinations, status);
  context.subscriptions.push(vscode.window.registerTreeDataProvider("sweetpad.build.view", schemes));
  context.subscriptions.push(vscode.window.registerTreeDataProvider("sweetpad.destinations.view", destinations));
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("sweetpad.remote")) refresh();
    }),
  );
  await vscode.commands.executeCommand("setContext", "sweetpad.enabled", true);
  await vscode.commands.executeCommand("setContext", "sweetpad.remoteMode", true);
  await vscode.commands.executeCommand("setContext", "sweetpad.devFeatures", true);
  updateStatus();
  status.show();

  registerMacCommands(register, client, refresh);

  const selectionArgs = (item?: SchemeItem, clean = false, testing = false) => {
    const args: string[] = [];
    const scheme = item?.scheme ?? config().get<string>("remote.scheme");
    const destination = config().get<string>("remote.destination");
    const configuration = config().get<string>(testing ? "testing.configuration" : "build.configuration");
    if (scheme) args.push("--scheme", scheme);
    if (destination && !clean) args.push("--destination", destination);
    if (configuration) args.push("--configuration", configuration);
    return args;
  };
  const run = async (verb: string[], name: string, item?: SchemeItem) => {
    await client.run([...verb, ...selectionArgs(item, verb[0] === "clean", verb[0] === "test")], name, projectRoot());
  };
  register("sweetpad.build.build", async (item?: SchemeItem) => run(["build"], "Build", item));
  register("sweetpad.build.launch", async (item?: SchemeItem) => run(["run"], "Build & Run", item));
  register("sweetpad.build.run", async (item?: SchemeItem) => run(["app", "launch"], "Run", item));
  register("sweetpad.build.test", async (item?: SchemeItem) => run(["test"], "Test", item));
  register("sweetpad.build.clean", async (item?: SchemeItem) => run(["clean"], "Clean", item));
  register("sweetpad.build.resolveDependencies", async () =>
    client.run(["dependency", "resolve"], "Resolve Dependencies", projectRoot()),
  );
  register("sweetpad.build.openXcode", async () => client.run(["open", "xcode"], "Open Xcode", projectRoot()));
  register("sweetpad.format.run", async () => client.run(["format"], "Format", projectRoot()));
  register("sweetpad.build.stop", () => client.stop());
  register("sweetpad.build.refreshSchemes", () => schemes.refresh());
  register("sweetpad.devices.refresh", () => destinations.refresh());
  register("sweetpad.simulators.refresh", () => destinations.refresh());
  register("sweetpad.build.setDefaultScheme", async (item?: SchemeItem) => {
    let scheme = item?.scheme;
    if (!scheme) {
      const mac = await client.ensureMac();
      if (!mac) return;
      const data = await client.query<{ schemes: Scheme[] }>(["scheme", "list", "--remote", mac], projectRoot());
      scheme = await vscode.window.showQuickPick(data.schemes.map((entry) => entry.name));
    }
    if (scheme) await config().update("remote.scheme", scheme, vscode.ConfigurationTarget.Workspace);
    refresh();
  });
  register("sweetpad.testing.setDefaultScheme", async (item?: SchemeItem) => {
    await vscode.commands.executeCommand("sweetpad.build.setDefaultScheme", item);
  });
  const selectConfiguration = async (key: "build.configuration" | "testing.configuration") => {
    const value = await vscode.window.showInputBox({
      prompt: "Remote Xcode build configuration (for example Debug or Release)",
      value: config().get<string>(key) || "",
    });
    if (value !== undefined) await config().update(key, value || undefined, vscode.ConfigurationTarget.Workspace);
  };
  register("sweetpad.build.selectConfiguration", async () => selectConfiguration("build.configuration"));
  register("sweetpad.testing.selectConfiguration", async () => selectConfiguration("testing.configuration"));
  register("sweetpad.destinations.select", async (item?: DestinationItem) => {
    let destination = item?.destination;
    if (!destination) {
      const mac = await client.ensureMac();
      if (!mac) return;
      const data = await client.query<{ destinations: Destination[] }>(["devices", "--remote", mac], projectRoot());
      const picked = await vscode.window.showQuickPick(
        data.destinations.map((entry) => ({
          label: entry.name,
          description: entry.os,
          destination: entry.destination,
        })),
      );
      destination = picked?.destination;
    }
    if (destination) await config().update("remote.destination", destination, vscode.ConfigurationTarget.Workspace);
    refresh();
  });
  register("sweetpad.destinations.selectForTesting", async (item?: DestinationItem) => {
    await vscode.commands.executeCommand("sweetpad.destinations.select", item);
  });
  register("sweetpad.simulators.openSimulator", async () => client.run(["simulator", "open"], "Open Simulator"));
  register("sweetpad.simulators.start", async (item?: DestinationItem) => {
    await client.run(["simulator", "boot", ...(item?.udid ? [item.udid] : [])], "Boot Simulator");
    destinations.refresh();
  });
  register("sweetpad.simulators.stop", async (item?: DestinationItem) => {
    await client.run(["simulator", "shutdown", ...(item?.udid ? [item.udid] : [])], "Shutdown Simulator");
    destinations.refresh();
  });

  const { registerRemoteFeatures } = await import("./features.js");
  const features = registerRemoteFeatures(context, client, register);

  // Contributed menus still expose Mac-only features. Give a clear answer
  // instead of VS Code's opaque "command not found" when one is clicked.
  const commands = context.extension.packageJSON?.contributes?.commands as { command: string }[] | undefined;
  for (const entry of commands ?? []) {
    if (entry.command.startsWith("sweetpad.") && !registered.has(entry.command)) {
      register(entry.command, () => {
        void vscode.window.showInformationMessage(
          "This action needs the Mac-local extension. Use SweetPad: Run Remote CLI Command for its CLI equivalent.",
        );
      });
    }
  }
  return { mode: "remote" as const, client, ...features };
}
