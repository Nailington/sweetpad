export type EnumerationNode = { kind: string; name: string; children?: EnumerationNode[] };
export type EnumeratedTest = { id: string; label: string };
export function enumeratedTests(values: EnumerationNode[]): EnumeratedTest[] {
  const found: EnumeratedTest[] = [];
  const visit = (node: EnumerationNode, parents: string[]) => {
    const components = node.kind === "plan" ? parents : [...parents, node.name];
    if (node.kind === "test") found.push({ id: components.join("/"), label: node.name });
    for (const child of node.children ?? []) visit(child, components);
  };
  for (const node of values) visit(node, []);
  return found;
}

export type TestOutcome = { id: string; result: string; duration?: string; name: string };
export function testOutcomes(value: unknown): TestOutcome[] {
  const found: TestOutcome[] = [];
  const visit = (node: unknown) => {
    if (!node || typeof node !== "object") return;
    const fields = node as Record<string, unknown>;
    if (fields.nodeType === "Test Case" && typeof fields.nodeIdentifier === "string") {
      let id = fields.nodeIdentifier;
      if (typeof fields.nodeIdentifierURL === "string") {
        id = new URL(fields.nodeIdentifierURL).pathname
          .split("/")
          .filter(Boolean)
          .slice(1)
          .map(decodeURIComponent)
          .join("/");
      }
      found.push({
        id,
        result: String(fields.result),
        duration: String(fields.duration ?? ""),
        name: String(fields.name ?? ""),
      });
    }
    for (const child of Object.values(fields)) {
      if (Array.isArray(child)) child.forEach(visit);
      else if (typeof child === "object") visit(child);
    }
  };
  visit(value);
  return found;
}
