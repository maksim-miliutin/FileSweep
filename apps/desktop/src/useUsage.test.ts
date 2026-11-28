import { expect, test } from "vitest";

import type { UsageFileView } from "./backend/types";
import { usedBy, yearEdges } from "./useUsage";

function used(times: Pick<UsageFileView, "lastUsedMs" | "modifiedMs" | "accessedMs" | "createdMs">): UsageFileView
{
	return { id: 0, path: "/a/b.txt", name: "b.txt", folder: "/a", size: 1, ...times };
}

test("a last use is told by the time it came from", () =>
{
	expect(usedBy(used({ lastUsedMs: 3, modifiedMs: 3, accessedMs: 2, createdMs: 1 }))).toBe("changed");
	expect(usedBy(used({ lastUsedMs: 3, modifiedMs: 1, accessedMs: null, createdMs: 3 }))).toBe("created");
	expect(usedBy(used({ lastUsedMs: 3, modifiedMs: 1, accessedMs: 3, createdMs: 2 }))).toBe("opened");
});

test("of two times at the same moment the surer one is told", () =>
{
	expect(usedBy(used({ lastUsedMs: 3, modifiedMs: 3, accessedMs: 3, createdMs: 3 }))).toBe("changed");
	expect(usedBy(used({ lastUsedMs: 3, modifiedMs: null, accessedMs: 3, createdMs: 3 }))).toBe("created");
});

test("the years begin on the first of January by the clock of this computer", () =>
{
	expect(yearEdges(new Date(2026, 5, 15), 3)).toEqual([
		new Date(2024, 0, 1).getTime(),
		new Date(2025, 0, 1).getTime(),
		new Date(2026, 0, 1).getTime(),
	]);
	expect(yearEdges(new Date(2026, 0, 1), 1)).toEqual([new Date(2026, 0, 1).getTime()]);
});
