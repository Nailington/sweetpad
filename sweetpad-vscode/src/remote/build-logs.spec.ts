import { describe, expect, it, vi } from "vitest";

import { extractRemoteBuildLog } from "./build-logs";
import type { RemoteClient } from "./extension";

describe("remote build log extraction", () => {
  it("returns signing output locally without selecting a scheme or building", async () => {
    const execute = vi.fn().mockResolvedValue({
      code: 0,
      stderr: "syncing project…",
      stdout: JSON.stringify({
        ok: true,
        data: { text: "CodeSign App.debug.dylib\nerrSecInternalComponent\n" },
      }),
    });
    const text = await extractRemoteBuildLog({ execute } as unknown as RemoteClient, "/project ü");
    expect(text).toContain("errSecInternalComponent");
    expect(execute).toHaveBeenCalledExactlyOnceWith(["--json", "build", "logs"], "/project ü");
  });

  it("surfaces the missing-bundle error instead of opening an empty log", async () => {
    const execute = vi.fn().mockResolvedValue({
      code: 1,
      stdout: "",
      stderr: "SSH notice\n" + JSON.stringify({ ok: false, error: { message: "no saved Xcode build result" } }),
    });
    await expect(extractRemoteBuildLog({ execute } as unknown as RemoteClient, "/project")).rejects.toThrow(
      "no saved Xcode build result",
    );
  });
});
