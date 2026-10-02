import { describe, it, expect } from "vitest";

import { RemotePaths } from "./paths";
import { MappedProtocol } from "./protocol";

describe("remote framed protocols", () => {
  it("maps UTF-8 messages split across every byte boundary", async () => {
    const stream = new MappedProtocol(new RemotePaths("/local", "/remote ü"), "remote");
    const output: Buffer[] = [];
    stream.on("data", (data) => output.push(data));
    const finished = new Promise<void>((resolve, reject) => {
      stream.on("end", resolve);
      stream.on("error", reject);
    });
    const body = Buffer.from(JSON.stringify({ id: 1, params: { uri: "file:///local/App.swift", text: "é😀" } }));
    const frame = Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]);
    for (const byte of frame) stream.write(Buffer.from([byte]));
    stream.end();
    await finished;
    const received = Buffer.concat(output);
    const delimiter = received.indexOf("\r\n\r\n");
    const parsed = JSON.parse(received.subarray(delimiter + 4).toString());
    expect(parsed.params.uri).toBe("file:///remote%20%C3%BC/App.swift");
    expect(parsed.params.text).toBe("é😀");
    expect(Number(/Content-Length: (\d+)/.exec(received.toString())![1])).toBe(received.length - delimiter - 4);
  });
  it("rejects incomplete frames on EOF", async () => {
    const stream = new MappedProtocol(new RemotePaths("/local", "/remote"), "remote");
    const failure = new Promise<Error>((resolve) => stream.on("error", resolve));
    stream.resume();
    stream.end("Content-Length: 100\r\n\r\n{}");
    expect((await failure).message).toContain("Truncated");
  });
});
