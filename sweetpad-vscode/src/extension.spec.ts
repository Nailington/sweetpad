import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  mac: undefined as string | undefined,
  remote: vi.fn(),
  local: vi.fn(),
}));
vi.mock("vscode", () => ({
  workspace: { getConfiguration: () => ({ get: () => mocks.mac }) },
}));
vi.mock("./remote/extension.js", () => ({ activateRemote: mocks.remote }));
vi.mock("./local-extension.js", () => ({ activate: mocks.local }));

describe("extension host platform selection", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    mocks.mac = undefined;
  });
  afterEach(() => vi.restoreAllMocks());

  it.each(["win32", "linux"] as const)(
    "activates the remote client on %s without the native addon",
    async (platform) => {
      vi.spyOn(process, "platform", "get").mockReturnValue(platform);
      const { activate } = await import("./extension.js");
      await activate({} as never);
      expect(mocks.remote).toHaveBeenCalledOnce();
      expect(mocks.local).not.toHaveBeenCalled();
    },
  );

  it("activates the remote client on a Mac with a saved remote selection", async () => {
    vi.spyOn(process, "platform", "get").mockReturnValue("darwin");
    mocks.mac = "Test Mac";
    const { activate } = await import("./extension.js");
    await activate({} as never);
    expect(mocks.remote).toHaveBeenCalledOnce();
    expect(mocks.local).not.toHaveBeenCalled();
  });

  it("activates the local client on a Mac without a remote selection", async () => {
    vi.spyOn(process, "platform", "get").mockReturnValue("darwin");
    const { activate } = await import("./extension.js");
    await activate({} as never);
    expect(mocks.local).toHaveBeenCalledOnce();
    expect(mocks.remote).not.toHaveBeenCalled();
  });
});
