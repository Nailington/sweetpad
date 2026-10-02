import { Transform, type TransformCallback } from "node:stream";

import type { RemotePaths } from "./paths";

/** Content-Length framing shared by LSP and DAP. Handles arbitrary splits and
 * UTF-8 byte lengths, rather than treating a stream chunk as one message. */
export class MappedProtocol extends Transform {
  private pending: Buffer = Buffer.alloc(0);
  constructor(
    private readonly paths: RemotePaths,
    private readonly direction: "local" | "remote",
  ) {
    super();
  }
  _transform(chunk: Buffer, _encoding: BufferEncoding, callback: TransformCallback): void {
    try {
      this.pending = Buffer.concat([this.pending, chunk]);
      while (true) {
        const end = this.pending.indexOf("\r\n\r\n");
        if (end < 0) {
          if (this.pending.length > 8192) throw new Error("Oversized protocol header");
          break;
        }
        const header = this.pending.subarray(0, end).toString("ascii");
        const match = /^Content-Length:\s*(\d+)\s*$/im.exec(header);
        if (!match) throw new Error("Missing protocol Content-Length");
        const length = Number(match[1]);
        if (length > 32 * 1024 * 1024) throw new Error("Oversized protocol message");
        if (this.pending.length < end + 4 + length) break;
        const message = JSON.parse(this.pending.subarray(end + 4, end + 4 + length).toString("utf8"));
        const body = Buffer.from(JSON.stringify(this.paths.message(message, this.direction)));
        this.push(Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]));
        this.pending = this.pending.subarray(end + 4 + length);
      }
      callback();
    } catch (error) {
      callback(error as Error);
    }
  }
  _flush(callback: TransformCallback): void {
    callback(this.pending.length ? new Error("Truncated protocol message") : undefined);
  }
}
