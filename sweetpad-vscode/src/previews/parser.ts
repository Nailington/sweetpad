/**
 * Lightweight scanner that finds SwiftUI previews in Swift source — both the
 * modern `#Preview` macro and the legacy `PreviewProvider` conformance.
 *
 * This is deliberately a line/regex scanner rather than a full SwiftSyntax
 * parse: SweetPad only needs the *location* of each preview (to place a
 * CodeLens and to build a navigable index), not a semantic understanding of
 * the view. It is intentionally conservative — it skips `//` line comments and
 * tolerates extra inheritance/trait arguments — and never throws on malformed
 * input.
 */

export type PreviewKind = "macro" | "provider";

export interface PreviewMatch {
  kind: PreviewKind;
  /**
   * Human label for the preview: the first string-literal argument of
   * `#Preview("…")`, or the type name for a `PreviewProvider`. `undefined` for
   * a bare `#Preview { … }`.
   */
  label?: string;
  /** 0-based line of the preview declaration. */
  line: number;
  /** 0-based column where the `#Preview` / declaration keyword starts. */
  character: number;
}

// `#Preview`, optionally followed by a parenthesised argument list. We capture
// the raw argument text so the caller can pull out the leading string label.
const MACRO_RE = /#Preview\b[ \t]*(\(([^)]*)\))?/;

// A `struct`/`class`/`enum` (or `extension`) whose inheritance clause contains
// `PreviewProvider`. The type name is captured; `final`/`public`/etc. modifiers
// before the keyword are allowed because the match doesn't anchor to column 0.
const PROVIDER_RE = /\b(?:struct|class|enum|extension)[ \t]+([A-Za-z_]\w*)[^{]*:[^{]*\bPreviewProvider\b/;

// First double-quoted string literal inside an argument list.
const FIRST_STRING_RE = /"((?:[^"\\]|\\.)*)"/;

/**
 * Mask strings and comments so their text cannot match a preview declaration.
 * Preserve line breaks and character offsets for the editor's source locations.
 */
function declarationText(text: string): string {
  const masked = text.split("");
  const blank = (start: number, end: number) => {
    for (let i = start; i < end; i++) if (masked[i] !== "\n") masked[i] = " ";
  };
  let i = 0;
  while (i < text.length) {
    const start = i;
    if (text.startsWith("//", i)) {
      const end = text.indexOf("\n", i);
      i = end < 0 ? text.length : end;
      blank(start, i);
      continue;
    }
    if (text.startsWith("/*", i)) {
      i += 2;
      let depth = 1;
      while (i < text.length && depth) {
        if (text.startsWith("/*", i)) {
          depth++;
          i += 2;
        } else if (text.startsWith("*/", i)) {
          depth--;
          i += 2;
        } else i++;
      }
      blank(start, i);
      continue;
    }
    let quote = i;
    while (text[quote] === "#") quote++;
    if (text[quote] === '"') {
      const hashes = text.slice(i, quote);
      const quotes = text.startsWith('"""', quote) ? '"""' : '"';
      const ending = quotes + hashes;
      i = quote + quotes.length;
      while (i < text.length) {
        if (text.startsWith(ending, i)) {
          i += ending.length;
          break;
        }
        if (text.startsWith("\\" + hashes, i)) {
          i += 2 + hashes.length;
        } else i++;
      }
      blank(start, Math.min(i, text.length));
      continue;
    }
    i++;
  }
  return masked.join("");
}

/**
 * Find every SwiftUI preview declaration in `text`, in source order.
 */
export function parsePreviews(text: string): PreviewMatch[] {
  const matches: PreviewMatch[] = [];
  const lines = text.split("\n");
  const codeLines = declarationText(text).split("\n");

  for (let lineNumber = 0; lineNumber < lines.length; lineNumber++) {
    const rawLine = lines[lineNumber];
    const line = codeLines[lineNumber];

    const macro = MACRO_RE.exec(line);
    if (macro) {
      const args = MACRO_RE.exec(rawLine.slice(macro.index))?.[2] ?? "";
      const label = FIRST_STRING_RE.exec(args)?.[1];
      matches.push({
        kind: "macro",
        label: label,
        line: lineNumber,
        character: macro.index,
      });
      continue;
    }

    const provider = PROVIDER_RE.exec(line);
    if (provider) {
      matches.push({
        kind: "provider",
        label: provider[1],
        line: lineNumber,
        character: provider.index,
      });
    }
  }

  return matches;
}

/**
 * Stable identifier for a preview, of the form `<relativePath>:<line>`. Used as
 * the key passed to the preview host so it knows which preview to render, and
 * as the de-dup key in the workspace index.
 */
export function previewId(relativePath: string, match: Pick<PreviewMatch, "line">): string {
  // 1-based line to match how editors and `#fileID:line` report locations.
  return `${relativePath}:${match.line + 1}`;
}
