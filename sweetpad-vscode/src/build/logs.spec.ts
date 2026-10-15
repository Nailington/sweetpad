import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  write: vi.fn(),
  open: vi.fn(),
  show: vi.fn(),
}));
vi.mock("vscode", () => ({
  Uri: { file: (fsPath: string) => ({ fsPath }) },
  workspace: { openTextDocument: mocks.open, fs: { writeFile: mocks.write } },
  window: { showSaveDialog: mocks.save, showTextDocument: mocks.show },
}));

import { buildResultPath, presentBuildLog } from "./logs";

describe("build log display and export", () => {
  beforeEach(() => vi.clearAllMocks());

  it("keeps same-named schemes in separate projects from overwriting each other's result", () => {
    const first = buildResultPath("/storage", "/first/App.xcodeproj", "App");
    const second = buildResultPath("/storage", "/second/App.xcodeproj", "App");
    expect(first).not.toBe(second);
    expect(first).toBe(buildResultPath("/storage", "/first/App.xcodeproj", "App"));
    expect(first).toMatch(/App\.xcresult$/);
  });

  it("opens the full extracted text as a log document", async () => {
    mocks.open.mockResolvedValue({ document: "log" });
    await presentBuildLog("codesign\nerrSecInternalComponent\n", "/project", false);
    expect(mocks.open).toHaveBeenCalledExactlyOnceWith({
      content: "codesign\nerrSecInternalComponent\n",
      language: "log",
    });
    expect(mocks.show).toHaveBeenCalledWith({ document: "log" }, { preview: false });
  });

  it("saves remote text through the local file provider with UTF-8 preserved", async () => {
    const uri = { fsPath: "/chosen folder/Maple.log" };
    mocks.save.mockResolvedValue(uri);
    await presentBuildLog("Compiling ü.swift\n", "/project", true);
    expect(mocks.write).toHaveBeenCalledExactlyOnceWith(uri, Buffer.from("Compiling ü.swift\n"));
    expect(mocks.show).toHaveBeenCalledWith(uri, { preview: false });
  });

  it("leaves files untouched when the save dialog is cancelled", async () => {
    mocks.save.mockResolvedValue(undefined);
    await presentBuildLog("log", "/project", true);
    expect(mocks.write).not.toHaveBeenCalled();
    expect(mocks.show).not.toHaveBeenCalled();
  });
});
