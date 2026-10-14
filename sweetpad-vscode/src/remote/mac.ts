import * as vscode from "vscode";

export type Mac = {
  name: string;
  host: string;
  port: number | null;
  identityFile?: string | null;
  auth?: "ssh" | "password";
};

/** Gather all changes before touching the saved Mac or workspace selection. */
export async function promptMac(existing?: Mac): Promise<Mac | undefined> {
  const name = await vscode.window.showInputBox({
    prompt: "Name for this Mac",
    value: existing?.name,
  });
  if (!name) return;
  const host = await vscode.window.showInputBox({
    prompt: "SSH target (user@host or SSH config alias)",
    value: existing?.host,
  });
  if (!host) return;
  const methods =
    existing?.auth === "password" ? ["Password", "SSH config / private key"] : ["SSH config / private key", "Password"];
  const method = await vscode.window.showQuickPick(methods, {
    placeHolder: "How should SweetPad authenticate to this Mac?",
  });
  if (!method) return;
  let key: string | undefined = "";
  if (method !== "Password") {
    key = await vscode.window.showInputBox({
      prompt: "Private key path (leave blank for your normal SSH config/agent)",
      value: existing?.identityFile ?? "",
    });
    if (key === undefined) return;
  }
  const port = await vscode.window.showInputBox({
    prompt: "SSH port (leave blank for SSH config/default)",
    value: existing?.port?.toString() ?? "",
  });
  if (port === undefined) return;
  if (port && (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535)) {
    throw new Error("SSH port must be between 1 and 65535.");
  }
  return {
    name,
    host,
    port: port ? Number(port) : null,
    identityFile: key || null,
    auth: method === "Password" ? "password" : "ssh",
  };
}

export function saveMacArgs(mac: Mac, replace = false): string[] {
  const args = ["remote", "add", mac.name, mac.host];
  if (replace) args.push("--replace");
  if (mac.auth === "password") args.push("--auth", "password");
  else if (mac.identityFile) args.push("--identity-file", mac.identityFile);
  if (mac.port !== null) args.push("--port", String(mac.port));
  return args;
}
