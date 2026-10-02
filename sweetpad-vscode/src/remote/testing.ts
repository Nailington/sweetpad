import * as vscode from "vscode";

import { targetArgs } from "./code-intelligence";
import type { RemoteClient } from "./extension";
import { projectRoot } from "./extension";
import { enumeratedTests, testOutcomes, type EnumerationNode } from "./test-results";

export class RemoteTesting implements vscode.Disposable {
  private swiftPackage = false;
  readonly controller = vscode.tests.createTestController("sweetpad-remote", "SweetPad Remote");
  constructor(private readonly client: RemoteClient) {
    this.controller.resolveHandler = async () => this.discover();
    this.controller.refreshHandler = async () => this.discover();
    this.controller.createRunProfile(
      "Run on Mac",
      vscode.TestRunProfileKind.Run,
      async (request, token) => this.run(request, token),
      true,
    );
  }
  async discover(): Promise<void> {
    const plan = await this.client.query<{ command: string[] }>(
      ["test", "--show-command", ...targetArgs(true), "--remote", this.client.mac!],
      projectRoot(),
    );
    const args = [...plan.command];
    this.swiftPackage = args[0] === "swift";
    if (this.swiftPackage) {
      args.splice(args.indexOf("test") + 1, 0, "list");
      const response = await this.client.execute(this.client.bridgeArgs("exec", args));
      if (response.code !== 0) throw new Error(response.stderr || response.stdout);
      const ids = response.stdout
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter((line) => line.includes("/") && !line.includes(" "));
      this.controller.items.replace(ids.map((id) => this.controller.createTestItem(id, id)));
      return;
    }
    const bundle = args.indexOf("-resultBundlePath");
    if (bundle >= 0) args.splice(bundle, 2);
    const result = await this.client.execute(
      this.client.bridgeArgs("exec", [
        ...args,
        "-quiet",
        "-enumerate-tests",
        "-test-enumeration-style",
        "hierarchical",
        "-test-enumeration-format",
        "json",
        "-test-enumeration-output-path",
        "-",
      ]),
    );
    if (result.code !== 0) throw new Error(result.stderr || result.stdout);
    const data = JSON.parse(result.stdout.slice(result.stdout.indexOf("{"))) as {
      errors: unknown[];
      values: EnumerationNode[];
    };
    if (data.errors?.length) throw new Error(JSON.stringify(data.errors));
    this.controller.items.replace(
      enumeratedTests(data.values).map((test) => this.controller.createTestItem(test.id, test.id)),
    );
  }
  async run(request: vscode.TestRunRequest, token: vscode.CancellationToken): Promise<void> {
    if (!this.controller.items.size) await this.discover();
    const run = this.controller.createTestRun(request);
    const items = request.include ?? [...this.controller.items].map(([, item]) => item);
    const excluded = new Set(request.exclude?.map((item) => item.id));
    const selected = items.filter((item) => !excluded.has(item.id));
    selected.forEach((item) => run.started(item));
    try {
      if (!selected.length) return;
      if (token.isCancellationRequested) {
        selected.forEach((item) => run.skipped(item));
        return;
      }
      if (this.swiftPackage) {
        // SwiftPM has no xcresult. Run each selected test with its exact filter
        // so an aggregate pass/fail result can be assigned without guessing.
        for (const item of selected) {
          if (token.isCancellationRequested) {
            run.skipped(item);
            continue;
          }
          const response = await this.client.execute(
            [
              "--json",
              "test",
              ...targetArgs(true),
              "--only-testing",
              `^${item.id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`,
            ],
            projectRoot(),
            token,
          );
          run.appendOutput(response.stderr.replace(/\r?\n/g, "\r\n"));
          if (token.isCancellationRequested) {
            run.skipped(item);
            continue;
          }
          const result = JSON.parse(response.stdout) as {
            ok: boolean;
            data?: { passed: boolean };
            error?: { message: string };
          };
          if (result.data?.passed) run.passed(item);
          else if (result.data)
            run.failed(item, new vscode.TestMessage(response.stderr || "Swift package test failed."));
          else run.errored(item, new vscode.TestMessage(result.error?.message || "Swift package test did not run."));
        }
        return;
      }
      const args = ["--json", "test", ...targetArgs(true)];
      for (const item of selected) args.push("--only-testing", item.id);
      const response = await this.client.execute(args, projectRoot(), token);
      run.appendOutput(response.stderr.replace(/\r?\n/g, "\r\n"));
      if (token.isCancellationRequested) {
        selected.forEach((item) => run.skipped(item));
        return;
      }
      const envelope = JSON.parse(response.stdout) as {
        data?: { resultBundle?: string; failures?: { test: string; message: string }[] };
      };
      if (!envelope.data?.resultBundle) throw new Error(response.stderr || "No remote result bundle returned.");
      const manifest = await this.client.execute(
        this.client.bridgeArgs("exec", [
          "xcrun",
          "xcresulttool",
          "get",
          "test-results",
          "tests",
          "--path",
          envelope.data.resultBundle,
        ]),
      );
      if (manifest.code !== 0) throw new Error(manifest.stderr);
      const outcomes = testOutcomes(JSON.parse(manifest.stdout));
      for (const item of selected) {
        const outcome = outcomes.find(
          (test) =>
            test.id.replace(/\(\)$/, " ").trim() === item.id.replace(/\(\)$/, " ").trim() ||
            item.id.endsWith(`/${test.id}`),
        );
        if (!outcome) {
          run.errored(item, new vscode.TestMessage("Test was not present in the result bundle."));
          continue;
        }
        if (outcome.result === "Passed") run.passed(item, parseFloat(outcome.duration ?? "") * 1000 || undefined);
        else if (outcome.result === "Skipped") run.skipped(item);
        else
          run.failed(
            item,
            new vscode.TestMessage(
              envelope.data.failures?.find((f) => item.id.includes(f.test))?.message || outcome.result,
            ),
          );
      }
    } catch (error) {
      selected.forEach((item) => run.errored(item, new vscode.TestMessage(String(error))));
    } finally {
      run.end();
    }
  }
  dispose(): void {
    this.controller.dispose();
  }
}
