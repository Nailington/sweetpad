import { beforeEach, expect, it, vi } from "vitest";

import type { RemoteClient } from "./extension";
const mocks = vi.hoisted(() => ({
  query: vi.fn(),
  execute: vi.fn(),
  started: vi.fn(),
  passed: vi.fn(),
  failed: vi.fn(),
  errored: vi.fn(),
  skipped: vi.fn(),
  end: vi.fn(),
}));
vi.mock("./extension", () => ({ projectRoot: () => "project" }));
vi.mock("./code-intelligence", () => ({ targetArgs: () => [] }));
vi.mock("vscode", () => ({
  TestRunProfileKind: { Run: 1 },
  TestMessage: class {
    constructor(public message: string) {}
  },
  tests: {
    createTestController: () => {
      const map = new Map<string, { id: string }>();
      const items = Object.assign(map, {
        add: (item: { id: string }) => map.set(item.id, item),
        replace: (values: { id: string }[]) => {
          map.clear();
          for (const item of values) map.set(item.id, item);
        },
      });
      return {
        items,
        createTestItem: (id: string, label: string) => ({ id, label }),
        createRunProfile: () => {},
        createTestRun: () => ({ ...mocks, appendOutput: () => {} }),
        dispose: () => {},
      };
    },
  },
}));
import { RemoteTesting } from "./testing";
const token = { isCancellationRequested: false } as never;
function testing() {
  return new RemoteTesting({
    mac: "Test",
    query: mocks.query,
    execute: mocks.execute,
    bridgeArgs: (_service: string, args: string[]) => args,
  } as unknown as RemoteClient);
}
beforeEach(() => {
  vi.clearAllMocks();
});
it("excluding every test does not accidentally run the whole Xcode suite", async () => {
  const service = testing();
  const item = service.controller.createTestItem("Target/Class/test()", "test");
  service.controller.items.add(item);
  await service.run({ exclude: [item] } as never, token);
  expect(mocks.execute).not.toHaveBeenCalled();
  expect(mocks.end).toHaveBeenCalledOnce();
});
it("an already cancelled run skips its cases without starting a command", async () => {
  const service = testing();
  const item = service.controller.createTestItem("Target/Class/test()", "test");
  service.controller.items.add(item);
  await service.run({} as never, { isCancellationRequested: true } as never);
  expect(mocks.execute).not.toHaveBeenCalled();
  expect(mocks.skipped).toHaveBeenCalledWith(item);
});
it("assigns Xcode outcomes individually instead of using the aggregate exit status", async () => {
  const service = testing();
  for (const id of ["Target/Class/pass()", "Target/Class/fail()"]) {
    const item = service.controller.createTestItem(id, id);
    service.controller.items.add(item);
  }
  mocks.execute
    .mockResolvedValueOnce({
      code: 1,
      stdout: JSON.stringify({ data: { resultBundle: "/remote/result.xcresult" } }),
      stderr: "test failed",
    })
    .mockResolvedValueOnce({
      code: 0,
      stdout: JSON.stringify({
        testNodes: [
          {
            nodeType: "Test Case",
            nodeIdentifier: "Class/pass()",
            result: "Passed",
            duration: "0.25s",
          },
          { nodeType: "Test Case", nodeIdentifier: "Class/fail()", result: "Failed" },
        ],
      }),
    });
  await service.run({} as never, token);
  expect(mocks.passed).toHaveBeenCalledWith(service.controller.items.get("Target/Class/pass()"), 250);
  expect(mocks.failed).toHaveBeenCalledOnce();
  expect(mocks.errored).not.toHaveBeenCalled();
});
it("escapes SwiftPM regex filters so similarly named tests cannot count as the selected test", async () => {
  const service = testing();
  mocks.query.mockResolvedValue({ command: ["swift", "test"] });
  mocks.execute.mockResolvedValueOnce({ code: 0, stdout: "Tests.Class/testValue()\n", stderr: "" });
  await service.discover();
  mocks.execute.mockResolvedValueOnce({
    code: 0,
    stdout: JSON.stringify({ data: { passed: true } }),
    stderr: "",
  });
  await service.run({} as never, token);
  expect(mocks.execute.mock.calls[1][0]).toContain("^Tests\\.Class/testValue\\(\\)$");
});
