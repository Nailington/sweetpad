import { createHash } from "node:crypto";
import path from "node:path";

import * as vscode from "vscode";

/** Same-named schemes in different projects must retain separate evidence. */
export function buildResultPath(storage: string, container: string, scheme: string): string {
  const project = createHash("sha256").update(container).digest("hex").slice(0, 16);
  return path.join(storage, "bundle", project, `${scheme}.xcresult`);
}

/** Shared UI for local and remote extraction. Never loads the Mac-only addon. */
export async function presentBuildLog(text: string, root: string, save: boolean): Promise<void> {
  if (save) {
    const uri = await vscode.window.showSaveDialog({
      title: "Save Xcode Build Log",
      defaultUri: vscode.Uri.file(path.join(root, "sweetpad-build.log")),
      filters: { "Build log": ["log", "txt"] },
    });
    if (!uri) return;
    await vscode.workspace.fs.writeFile(uri, Buffer.from(text, "utf8"));
    await vscode.window.showTextDocument(uri, { preview: false });
  } else {
    const document = await vscode.workspace.openTextDocument({ content: text, language: "log" });
    await vscode.window.showTextDocument(document, { preview: false });
  }
}
