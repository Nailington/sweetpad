import { it, expect } from "vitest";

import { enumeratedTests, testOutcomes } from "./test-results";
it("preserves target/class/method identity from enumeration to xcresult", () => {
  const enumeration = enumeratedTests([
    {
      kind: "plan",
      name: "Maple",
      children: [
        {
          kind: "target",
          name: "MapleTests",
          children: [{ kind: "class", name: "MapleTests", children: [{ kind: "test", name: "example()" }] }],
        },
      ],
    },
  ]);
  const results = testOutcomes({
    testNodes: [
      {
        nodeType: "Test Case",
        nodeIdentifier: "MapleTests/example()",
        nodeIdentifierURL: "test://com.apple.xcode/Maple/MapleTests/MapleTests/example()",
        result: "Passed",
      },
    ],
  });
  expect(enumeration[0].id).toBe("MapleTests/MapleTests/example()");
  expect(results[0].id).toBe(enumeration[0].id);
});
