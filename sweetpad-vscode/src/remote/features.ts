import * as vscode from "vscode";

import type { PreviewItem } from "../previews/types";
import { RemoteCodeIntelligence, symbolSearchCommands, targetArgs } from "./code-intelligence";
import type { RemoteClient } from "./extension";
import { config, projectRoot } from "./extension";
import { RemotePreviews } from "./previews";
import { RemoteStreams } from "./stream";
import { RemoteTesting } from "./testing";

export function registerRemoteFeatures(
  context: vscode.ExtensionContext,
  client: RemoteClient,
  register: (id: string, action: (...args: any[]) => Promise<void> | void) => void,
) {
  const intelligence = new RemoteCodeIntelligence(client);
  const testing = new RemoteTesting(client);
  const streams = new RemoteStreams(client);
  const previews = new RemotePreviews(client, streams);
  intelligence.register(context);
  previews.register(context);
  context.subscriptions.push(testing, streams);
  register("sweetpad.bsp.setup", () => intelligence.startLanguageServer());
  register("sweetpad.bsp.doctor", async () => {
    const data = await client.query(["bsp", "doctor", "--remote", client.mac!], projectRoot());
    intelligence.output.appendLine(JSON.stringify(data, null, 2));
    intelligence.output.show();
  });
  register("sweetpad.bsp.showLogs", () => intelligence.output.show());
  register("sweetpad.debugger.debuggingLaunch", async () => {
    await vscode.debug.startDebugging(vscode.workspace.workspaceFolders?.[0], {
      type: "sweetpad-remote",
      request: "attach",
      name: "SweetPad: Debug on Mac",
    });
  });
  register("sweetpad.debugger.debuggingRun", async () => {
    const launched = await client.query<{ pid: number; executable?: string }>(
      ["app", "launch", "--wait-for-debugger", ...targetArgs(), "--remote", client.mac!],
      projectRoot(),
    );
    await vscode.debug.startDebugging(vscode.workspace.workspaceFolders?.[0], {
      type: "sweetpad-remote",
      request: "attach",
      name: "SweetPad: Debug on Mac",
      pid: launched.pid,
      program: launched.executable,
      initCommands: symbolSearchCommands(launched.executable),
      stopOnEntry: true,
    });
  });
  register("sweetpad.debugger.debuggingBuild", async () =>
    client.run(["build", ...targetArgs()], "Build for Debugging", projectRoot()),
  );
  register("sweetpad.testing.selectTarget", () => testing.discover());
  for (const [id, action] of [
    ["buildForTesting", "build-for-testing"],
    ["testWithoutBuilding", "test-without-building"],
  ]) {
    register(`sweetpad.testing.${id}`, async () => {
      const plan = await client.query<{ command: string[] }>(
        ["test", "--show-command", ...targetArgs(true), "--remote", client.mac!],
        projectRoot(),
      );
      const args = [...plan.command];
      if (args[0] === "swift") {
        if (action === "build-for-testing") args.splice(args.indexOf("test"), 1, "build", "--build-tests");
        else args.splice(args.indexOf("test") + 1, 0, "--skip-build");
      } else {
        const index = args.indexOf("test");
        if (index < 0) throw new Error("The remote test plan did not contain an Xcode test action.");
        args[index] = action;
        if (action === "build-for-testing") {
          const resultBundle = args.indexOf("-resultBundlePath");
          if (resultBundle >= 0) args.splice(resultBundle, 2);
        }
      }
      await client.run(["bridge", "exec", "--", ...args], action, projectRoot());
    });
  }
  const simulator = async (item?: { udid?: string }) => item?.udid ?? (await previews.simulator());
  register("sweetpad.simulators.stream", async (item) => streams.show(await simulator(item)));
  register("sweetpad.simulators.streamOpenInBrowser", async (item) => {
    const stream = await streams.start(await simulator(item));
    await vscode.env.openExternal(await vscode.env.asExternalUri(vscode.Uri.parse(stream.url)));
  });
  register("sweetpad.simulators.streamCopyUrl", async (item) => {
    const stream = await streams.start(await simulator(item));
    await vscode.env.clipboard.writeText((await vscode.env.asExternalUri(vscode.Uri.parse(stream.url))).toString());
  });
  register("sweetpad.previews.setup", () => previews.setup());
  register("sweetpad.previews.refresh", () => previews.manager.refresh());
  register("sweetpad.previews.render", async (item?: PreviewItem) => {
    const selected = await previews.choose(item);
    if (selected) await previews.render(selected);
  });
  register("sweetpad.previews.screenshot", async (item?: PreviewItem) => {
    const selected = await previews.choose(item);
    if (selected) await previews.screenshot(selected);
  });
  register("sweetpad.previews.screenshotVariants", async (item?: PreviewItem) => {
    const selected = await previews.choose(item);
    if (selected) await previews.screenshot(selected, true);
  });
  const start = () => {
    if (client.mac && config().get<boolean>("remote.languageServer.enabled") !== false) {
      void intelligence
        .startLanguageServer()
        .catch((error) => intelligence.output.appendLine(`SourceKit startup failed: ${String(error)}`));
    } else {
      void intelligence.stopLanguageServer().catch((error) => intelligence.output.appendLine(String(error)));
    }
  };
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (
        ["sweetpad.remote.mac", "sweetpad.remote.streamCommand", "sweetpad.remote.streamArgs"].some((key) =>
          event.affectsConfiguration(key),
        )
      )
        streams.stopAll();
      if (
        [
          "sweetpad.remote.mac",
          "sweetpad.remote.languageServer",
          "sweetpad.remote.developerDir",
          "sweetpad.remote.cliPath",
        ].some((key) => event.affectsConfiguration(key))
      )
        start();
    }),
  );
  start();
  return { intelligence, testing, streams, previews };
}
