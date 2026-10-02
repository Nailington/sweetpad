import path from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";

// Map only whole path prefixes. Never replace an incidental substring in source
// text, diagnostics or LLDB expressions.
export class RemotePaths {
  readonly localRoot: string;
  constructor(
    localRoot: string,
    readonly remoteRoot: string,
    private readonly externalFiles = false,
  ) {
    this.localRoot = localRoot.replace(/^\\\\\?\\UNC\\/i, "\\\\").replace(/^\\\\\?\\/, "");
  }

  toRemote(value: string): string {
    if (this.externalFiles && value.startsWith("sweetpad-remote:")) return value.replace(/^sweetpad-remote:/, "file:");
    return this.map(value, this.localRoot, this.remoteRoot, true);
  }

  toLocal(value: string): string {
    return this.map(value, this.remoteRoot, this.localRoot, false);
  }

  private map(value: string, from: string, to: string, remote: boolean): string {
    const uri = value.startsWith("file:");
    let source = value;
    if (uri) {
      try {
        const parsed = new URL(value);
        source =
          !remote || (parsed.hostname === "" && !/^\/[a-z]:\//i.test(decodeURIComponent(parsed.pathname)))
            ? decodeURIComponent(parsed.pathname)
            : fileURLToPath(value);
      } catch {
        return value;
      }
    }
    // macOS and VS Code can report decomposed Unicode filenames. Compare a
    // canonical form so the same project does not become an external SDK URI.
    const normalized = source
      .replace(/^\\\\\?\\UNC\\/i, "\\\\")
      .replace(/^\\\\\?\\/, "")
      .replaceAll("\\", "/")
      .normalize("NFC");
    const base = from
      .replace(/^\\\\\?\\/, "")
      .replaceAll("\\", "/")
      .replace(/\/$/, "")
      .normalize("NFC");
    const comparable = remote && process.platform === "win32" ? normalized.toLowerCase() : normalized;
    const prefix = remote && process.platform === "win32" ? base.toLowerCase() : base;
    if (comparable !== prefix && !comparable.startsWith(`${prefix}/`)) {
      if (!remote && uri && this.externalFiles && source.startsWith("/")) {
        return value.replace(/^file:/, "sweetpad-remote:");
      }
      return value;
    }
    const suffix = normalized.slice(base.length).replace(/^\//, "");
    const mapped = remote ? path.posix.join(to, suffix) : path.join(to, suffix);
    if (!uri) return mapped;
    return remote
      ? new URL(`file://${mapped.split("/").map(encodeURIComponent).join("/")}`).href
      : pathToFileURL(mapped).href;
  }

  message<T>(value: T, direction: "local" | "remote"): T {
    if (typeof value === "string") return (direction === "remote" ? this.toRemote(value) : this.toLocal(value)) as T;
    if (Array.isArray(value)) return value.map((item) => this.message(item, direction)) as T;
    if (value !== null && typeof value === "object") {
      return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, this.message(item, direction)])) as T;
    }
    return value;
  }
}
