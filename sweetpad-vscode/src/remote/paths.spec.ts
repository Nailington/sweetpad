import path from "node:path";
import { pathToFileURL } from "node:url";

import { describe, it, expect } from "vitest";

import { RemotePaths } from "./paths";

describe("remote source paths", () => {
  const local = path.resolve("project space ü");
  const mapping = new RemotePaths(local, "/Users/dev/remote workspace");
  it("round trips filenames and escaped file URIs", () => {
    const file = path.join(local, "Sources", "App ü.swift");
    expect(mapping.toLocal(mapping.toRemote(file))).toBe(file);
    const uri = pathToFileURL(file).href;
    expect(mapping.toLocal(mapping.toRemote(uri))).toBe(uri);
    expect(mapping.toRemote(uri)).toContain("file:///Users/dev/remote%20workspace/");
  });
  it("keeps adjacent directories and diagnostic text unchanged", () => {
    expect(mapping.toRemote(`${local}Other/file.swift`)).toBe(`${local}Other/file.swift`);
    expect(mapping.toRemote(`error in ${local}/file.swift`)).toBe(`error in ${local}/file.swift`);
  });
  it("maps nested protocol payloads without changing IDs or other values", () => {
    expect(mapping.message({ id: 7, params: { uri: pathToFileURL(local).href }, result: null }, "remote")).toEqual({
      id: 7,
      params: { uri: "file:///Users/dev/remote%20workspace" },
      result: null,
    });
  });
  it("opens SDK definitions through the read-only Mac document provider", () => {
    const paths = new RemotePaths(local, "/Users/dev/remote workspace", true);
    const sdk = "file:///Applications/Xcode.app/SDKs/Swift%20module.swiftinterface";
    expect(paths.toLocal(sdk)).toBe(sdk.replace("file:", "sweetpad-remote:"));
    expect(paths.toRemote(paths.toLocal(sdk))).toBe(sdk);
    expect(mapping.toLocal(sdk)).toBe(sdk);
  });
  it("maps decomposed Unicode paths returned by editors and macOS", () => {
    const paths = new RemotePaths(local, "/Users/dev/remote workspace", true);
    const file = path.join(local, "App ü.swift");
    expect(paths.toLocal(paths.toRemote(pathToFileURL(file.normalize("NFD")).href))).toBe(pathToFileURL(file).href);
  });
  it.skipIf(process.platform !== "win32")("normalizes extended-length drive and UNC roots", () => {
    const drive = new RemotePaths("\\\\?\\C:\\Project ü", "/remote");
    expect(drive.toLocal("file:///remote/App.swift")).toBe("file:///C:/Project%20%C3%BC/App.swift");
    expect(drive.toRemote("\\\\?\\C:\\Project ü\\App.swift")).toBe("/remote/App.swift");
    expect(drive.toRemote("file:///c%3A/Project%20%C3%BC/App.swift")).toBe("file:///remote/App.swift");
    const network = new RemotePaths("\\\\?\\UNC\\server\\share\\Project", "/remote");
    expect(network.toRemote("file://server/share/Project/App.swift")).toBe("file:///remote/App.swift");
  });
});
