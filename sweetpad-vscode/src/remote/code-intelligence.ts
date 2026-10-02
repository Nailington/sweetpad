import type { ChildProcess } from "node:child_process";
import path from "node:path";

import * as vscode from "vscode";
import type { Message } from "vscode-jsonrpc";
import { StreamMessageReader, StreamMessageWriter } from "vscode-jsonrpc/node";
import { LanguageClient } from "vscode-languageclient/node";

import type { RemoteClient } from "./extension";
import { config, projectRoot } from "./extension";
import { RemotePaths } from "./paths";
import { MappedProtocol } from "./protocol";

export async function remotePaths(client: RemoteClient, externalFiles = false): Promise<RemotePaths> {
  const context = await client.query<{ localRoot: string; remoteRoot: string }>(
    client.bridgeArgs("context"),
    projectRoot(),
  );
  return new RemotePaths(context.localRoot, context.remoteRoot, externalFiles);
}

export function targetArgs(testing = false): string[] {
  const args: string[] = [];
  for (const [flag, key] of [
    ["--scheme", "remote.scheme"],
    ["--destination", "remote.destination"],
    ["--configuration", testing ? "testing.configuration" : "build.configuration"],
  ]) {
    const value = config().get<string>(key);
    if (value) args.push(flag, value);
  }
  return args;
}

export function symbolSearchCommands(executable?: string): string[] {
  if (!executable) return [];
  const directory = path.posix.dirname(path.posix.dirname(executable));
  const quoted = `"${directory.replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
  return [`settings append target.debug-file-search-paths ${quoted}`];
}

export class RemoteCodeIntelligence implements vscode.Disposable {
  private languageClient?: LanguageClient;
  private languageProcess?: ChildProcess;
  private disposed = false;
  private starting: Promise<void> = Promise.resolve();
  readonly output = vscode.window.createOutputChannel("SweetPad Remote Services");
  private syncing: Promise<unknown> = Promise.resolve();
  constructor(private readonly client: RemoteClient) {}

  async startLanguageServer(): Promise<void> {
    this.starting = this.starting.catch(() => {}).then(() => this.startServer());
    return this.starting;
  }

  private async startServer(): Promise<void> {
    if (this.disposed) return;
    await this.languageClient?.stop();
    if (this.languageProcess) this.client.stopBridge(this.languageProcess);
    const paths = await remotePaths(this.client, true);
    if (this.disposed) return;
    const child = this.client.spawnBridge("lsp");
    this.languageProcess = child;
    child.stderr.on("data", (data) => this.output.append(data.toString()));
    child.on("error", (error) => this.output.appendLine(error.message));
    const incoming = new MappedProtocol(paths, "local");
    const outgoing = new MappedProtocol(paths, "remote");
    for (const stream of [incoming, outgoing])
      stream.on("error", (error) => {
        this.output.appendLine(`SourceKit transport failed: ${error.message}`);
        this.client.stopBridge(child);
      });
    child.stdout.pipe(incoming);
    outgoing.pipe(child.stdin);
    this.languageClient = new LanguageClient(
      "sweetpad-remote-sourcekit",
      "SweetPad Remote SourceKit",
      async () => ({
        reader: new StreamMessageReader(incoming),
        writer: new StreamMessageWriter(outgoing),
      }),
      {
        documentSelector: [
          { scheme: "file", language: "swift" },
          { scheme: "sweetpad-remote", language: "swift" },
        ],
        workspaceFolder: vscode.workspace.workspaceFolders?.[0],
        outputChannel: this.output,
      },
    );
    this.languageClient.registerFeature({
      fillClientCapabilities: (capabilities) => {
        capabilities.experimental = {
          ...capabilities.experimental,
          "sourcekit/workspace/getReferenceDocument": { supported: true },
          "workspace/getReferenceDocument": true,
        };
      },
      initialize: () => {},
      clear: () => {},
      getState: () => ({ kind: "static" }),
    });
    try {
      await this.languageClient.start();
    } catch (error) {
      child.stdin.end();
      child.kill();
      throw error;
    }
    // JSON-RPC shutdown/exit closes stdio and the SSH-owned service. Keep this
    // listener attached so a server crash never leaks the transport process.
    child.on("exit", () => {
      incoming.end();
      outgoing.destroy();
    });
  }

  syncOnSave(): void {
    this.syncing = this.syncing
      .catch(() => {})
      .then(() => remotePaths(this.client))
      .catch((error) => this.output.appendLine(`Save sync failed: ${String(error)}`));
  }

  register(context: vscode.ExtensionContext): void {
    context.subscriptions.push(
      this,
      vscode.workspace.registerTextDocumentContentProvider("sourcekit-lsp", {
        provideTextDocumentContent: async (uri, token) => {
          const client = this.languageClient;
          if (!client?.isRunning())
            throw new Error("Start the remote SourceKit language server to open this interface.");
          try {
            const result = await client.sendRequest<{ content: string }>(
              "sourcekit/workspace/getReferenceDocument",
              { uri: uri.toString(true) },
              token,
            );
            return result.content;
          } catch (error) {
            if ((error as { code?: number }).code !== -32601) throw error;
            const result = await client.sendRequest<{ content: string }>(
              "workspace/getReferenceDocument",
              { uri: uri.toString(true) },
              token,
            );
            return result.content;
          }
        },
      }),
      vscode.workspace.registerTextDocumentContentProvider("sweetpad-remote", {
        provideTextDocumentContent: async (uri, token) => {
          const file = decodeURIComponent(uri.path);
          if (!file.startsWith("/") || file.includes("\0")) throw new Error("Invalid remote source file.");
          const response = await this.client.execute(
            this.client.bridgeArgs("exec", ["cat", "--", file]),
            projectRoot(),
            token,
          );
          if (response.code !== 0) throw new Error(response.stderr || "Cannot read remote source file.");
          return response.stdout;
        },
      }),
      vscode.workspace.onDidSaveTextDocument((document) => {
        if (document.languageId === "swift") this.syncOnSave();
      }),
      vscode.debug.registerDebugConfigurationProvider("sweetpad-remote", {
        provideDebugConfigurations: () => [
          { type: "sweetpad-remote", request: "attach", name: "SweetPad: Debug on Mac" },
        ],
        resolveDebugConfiguration: async (_folder, configuration) => {
          if (configuration.program || configuration.pid) return configuration;
          const launched = await this.client.query<{ pid: number; executable?: string }>(
            ["run", "--detach", "--wait-for-debugger", ...targetArgs(), "--remote", this.client.mac!],
            projectRoot(),
          );
          if (!launched.pid) throw new Error("The remote app did not report a process to debug.");
          return {
            ...configuration,
            type: "sweetpad-remote",
            request: "attach",
            pid: launched.pid,
            program: launched.executable,
            initCommands: [...symbolSearchCommands(launched.executable), ...(configuration.initCommands ?? [])],
            stopOnEntry: true,
          };
        },
      }),
      vscode.debug.registerDebugAdapterDescriptorFactory("sweetpad-remote", {
        createDebugAdapterDescriptor: async () => {
          const paths = await remotePaths(this.client);
          const child = this.client.spawnBridge("dap");
          const messages = new vscode.EventEmitter<any>();
          const reader = new StreamMessageReader(child.stdout);
          const writer = new StreamMessageWriter(child.stdin);
          reader.listen((message) => messages.fire(paths.message(message, "local")));
          child.stderr.on("data", (data) => this.output.append(data.toString()));
          child.on("error", (error) => this.output.appendLine(error.message));
          writer.onError(([error]) => this.output.appendLine(error.message));
          return new vscode.DebugAdapterInlineImplementation({
            onDidSendMessage: messages.event,
            handleMessage: (message) => {
              void writer
                .write(paths.message(message, "remote") as unknown as Message)
                .catch((error) => this.output.appendLine(String(error)));
            },
            dispose: () => {
              writer.end();
              reader.dispose();
              messages.dispose();
              this.client.stopBridge(child);
            },
          });
        },
      }),
      vscode.debug.registerDebugConfigurationProvider("sweetpad-lldb", {
        resolveDebugConfiguration: () => ({
          type: "sweetpad-remote",
          request: "attach",
          name: "SweetPad: Debug on Mac",
        }),
      }),
    );
  }

  dispose(): void {
    this.disposed = true;
    void this.stopLanguageServer()
      .catch(() => {})
      .finally(() => this.output.dispose());
  }

  async stopLanguageServer(): Promise<void> {
    await this.starting.catch(() => {});
    await this.languageClient?.stop();
    if (this.languageProcess) this.client.stopBridge(this.languageProcess);
    this.languageProcess = undefined;
    this.languageClient = undefined;
  }
}
