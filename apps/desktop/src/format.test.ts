import { expect, test } from "vitest";

import { formatBytes, formatCount } from "./format";

test("a size is told the way Windows tells it", () =>
{
	expect(formatBytes(0)).toBe("0 bytes");
	expect(formatBytes(1)).toBe("1 byte");
	expect(formatBytes(1023)).toBe("1,023 bytes");
	expect(formatBytes(1024)).toBe("1 KB");
	expect(formatBytes(1536)).toBe("1.5 KB");
	expect(formatBytes(1_048_576)).toBe("1 MB");
	expect(formatBytes(734_003_200)).toBe("700 MB");
	expect(formatBytes(5 * 1024 ** 3)).toBe("5 GB");
	expect(formatBytes(3 * 1024 ** 5)).toBe("3 PB");
});

test("a size just under a unit is not told as 1,024 of the smaller one", () =>
{
	expect(formatBytes(1024 * 1024 - 1)).toBe("1 MB");
	expect(formatBytes(1024 ** 3 - 1)).toBe("1 GB");
});

test("a size past the largest unit stays in that unit", () =>
{
	expect(formatBytes(4096 * 1024 ** 5)).toBe("4,096 PB");
});

test("a count is grouped by thousands", () =>
{
	expect(formatCount(0)).toBe("0");
	expect(formatCount(1_234_567)).toBe("1,234,567");
});
