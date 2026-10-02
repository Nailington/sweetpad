import * as vscode from "vscode";

let remoteApi: Awaited<ReturnType<typeof import("./remote/extension.js").activateRemote>> | undefined;

// Keep the Mac-only native resolver out of Windows and Linux extension hosts. The remote
// UI uses the standalone CLI, which resolves project files on the Mac.
export async function activate(context: vscode.ExtensionContext) {
  const remote = vscode.workspace.getConfiguration("sweetpad").get<string>("remote.mac");
  if (process.platform !== "darwin" || remote) {
    const { activateRemote } = await import("./remote/extension.js");
    remoteApi = await activateRemote(context);
    return remoteApi;
  }
  const { activate: activateLocal } = await import("./local-extension.js");
  return await activateLocal(context);
}

export async function deactivate() {
  if (!remoteApi) return;
  remoteApi.streams.dispose();
  remoteApi.previews.dispose();
  await remoteApi.intelligence.stopLanguageServer();
  await remoteApi.client.shutdown();
  remoteApi = undefined;
}
