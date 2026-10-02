import type { ChildProcess } from "node:child_process";
import { get } from "node:http";
import { createServer } from "node:net";

import * as vscode from "vscode";

import type { RemoteClient } from "./extension";
import { config } from "./extension";

async function freePort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Could not allocate simulator stream port");
  await new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  return address.port;
}
function reachable(url: string): Promise<boolean> {
  return new Promise((resolve) => {
    const request = get(url, (response) => {
      response.resume();
      resolve(response.statusCode === 200);
    });
    request.setTimeout(1500, () => {
      request.destroy();
      resolve(false);
    });
    request.on("error", () => resolve(false));
  });
}
type Stream = { child: ChildProcess; url: string; panel?: vscode.WebviewPanel };
export class RemoteStreams implements vscode.Disposable {
  private streams = new Map<string, Stream>();
  private starting = new Map<string, Promise<Stream>>();
  private disposed = false;
  private generation = 0;
  constructor(private readonly client: RemoteClient) {}

  async start(udid: string): Promise<Stream> {
    if (this.disposed) throw new Error("Simulator streaming is disposed.");
    const pending = this.starting.get(udid);
    if (pending) return pending;
    const existing = this.streams.get(udid);
    if (existing) return existing;
    const promise = this.startService(udid);
    this.starting.set(udid, promise);
    try {
      return await promise;
    } finally {
      if (this.starting.get(udid) === promise) this.starting.delete(udid);
    }
  }
  private async startService(udid: string): Promise<Stream> {
    const generation = this.generation;
    const localPort = await freePort();
    if (this.disposed) throw new Error("Simulator streaming is disposed.");
    if (generation !== this.generation) throw new Error("Simulator stream was stopped.");
    const remotePort = 32000 + Math.floor(Math.random() * 20000);
    const command = config().get<string>("remote.streamCommand") || "npx";
    const args = config().get<string[]>("remote.streamArgs") ?? ["--yes", "serve-sim@0.1.47"];
    const child = this.client.spawnBridge("stream", [udid, String(localPort), String(remotePort), command, ...args]);
    const stream: Stream = { child, url: `http://127.0.0.1:${localPort}` };
    let failure: string | undefined,
      log = "";
    child.stdout.on("data", (data) => {
      log = (log + data.toString()).slice(-8192);
    });
    child.stderr.on("data", (data) => {
      log = (log + data.toString()).slice(-8192);
    });
    child.on("error", (error) => {
      failure = error.message;
    });
    child.on("exit", (code) => {
      failure = `Simulator stream exited (${code}): ${log}`;
      if (this.streams.get(udid) === stream) this.streams.delete(udid);
      stream.panel?.dispose();
    });
    this.streams.set(udid, stream);
    try {
      const deadline = Date.now() + 120000;
      while (Date.now() < deadline) {
        if (failure) throw new Error(failure);
        if (this.streams.get(udid) !== stream) throw new Error("Simulator stream was stopped.");
        if (await reachable(stream.url)) return stream;
        await new Promise((resolve) => setTimeout(resolve, 300));
      }
      throw new Error(failure || `Simulator stream did not become ready: ${log}`);
    } catch (error) {
      if (this.streams.get(udid) === stream) this.stop(udid);
      throw error;
    }
  }
  async show(udid: string): Promise<void> {
    const stream = await this.start(udid);
    if (stream.panel) {
      stream.panel.reveal(vscode.ViewColumn.Beside);
      return;
    }
    const uri = await vscode.env.asExternalUri(vscode.Uri.parse(stream.url));
    const panel = vscode.window.createWebviewPanel(
      "sweetpad.remote.simulator",
      "Remote Simulator",
      vscode.ViewColumn.Beside,
      { enableScripts: true, retainContextWhenHidden: true },
    );
    const url = uri.toString().replaceAll("&", "&amp;").replaceAll('"', "&quot;");
    panel.webview.html = `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="default-src 'none'; frame-src ${url}; style-src 'unsafe-inline'"><style>body{margin:0}iframe{border:0;width:100vw;height:100vh}</style></head><body><iframe title="Remote simulator" src="${url}"></iframe></body></html>`;
    stream.panel = panel;
    panel.onDidDispose(() => this.stop(udid));
  }
  stop(udid: string): void {
    const stream = this.streams.get(udid);
    if (!stream) return;
    this.streams.delete(udid);
    this.client.stopBridge(stream.child);
    stream.panel?.dispose();
  }
  stopAll(): void {
    this.generation++;
    this.starting.clear();
    for (const udid of this.streams.keys()) this.stop(udid);
  }
  dispose(): void {
    this.disposed = true;
    this.stopAll();
  }
}
