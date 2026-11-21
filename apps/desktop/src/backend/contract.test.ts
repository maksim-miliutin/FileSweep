import { describe, expect, test } from "vitest";

import { contract, contractObject } from "../test-contract";
import { DecodeError, decodeError, decodeGroupPage, decodeSkipPage, decodeStatus } from "./decode";
import { BackendError, PHASES, SCAN_STATES, SKIP_REASONS } from "./types";

// A decoder builds its answer field by field. Being equal to the example
// therefore means two things: nothing the interface needs is missing from
// what Rust sends, and Rust sends nothing the interface does not know about.
describe("the examples both sides are held to", () =>
{
	test.each([
		"status-idle.json",
		"status-scanning.json",
		"status-completed.json",
		"status-failed.json",
	])("%s decodes to exactly itself", (name) =>
	{
		expect(decodeStatus(contract(name))).toEqual(contract(name));
	});

	test("the group page decodes to exactly itself", () =>
	{
		expect(decodeGroupPage(contract("groups-page.json"))).toEqual(contract("groups-page.json"));
	});

	test("the skipped page decodes to exactly itself", () =>
	{
		expect(decodeSkipPage(contract("skipped-page.json"))).toEqual(contract("skipped-page.json"));
	});

	// This is what makes the comparisons above notice a field added on the
	// Rust side only: the decoder leaves it out, and the answer stops being
	// equal to the example.
	test("a field the interface does not know is left out", () =>
	{
		const status = { ...contractObject("status-idle.json"), surprise: 1 };

		expect(decodeStatus(status)).toEqual(contract("status-idle.json"));
	});

	test("the names the contract lists are the names the types know", () =>
	{
		const names = contractObject("names.json");
		const skipped = decodeSkipPage(contract("skipped-page.json"));

		expect(names.states).toEqual([...SCAN_STATES]);
		expect(names.phases).toEqual([...PHASES]);
		expect(skipped.items.map(item => item.reason)).toEqual([...SKIP_REASONS]);
	});

	test("every refusal becomes an error with its code and its details", () =>
	{
		const examples = contractObject("errors.json");

		for (const [name, example] of Object.entries(examples))
		{
			const error = decodeError(example);

			expect(error, name).toBeInstanceOf(BackendError);
			expect({ code: error.code, ...error.detail }, name).toEqual(example);
		}
	});
});

describe("a message of the wrong shape", () =>
{
	function scanning(): { state: unknown; scan: Record<string, unknown> }
	{
		return structuredClone(contract("status-scanning.json")) as { state: unknown; scan: Record<string, unknown> };
	}

	test("is refused with the name of the missing field", () =>
	{
		const status = scanning();
		delete status.scan.filesSeen;

		expect(() => decodeStatus(status)).toThrow(DecodeError);
		expect(() => decodeStatus(status)).toThrow("status.scan.filesSeen");
	});

	test("is refused when the state is one the interface does not know", () =>
	{
		const status = scanning();
		status.state = "sleeping";

		expect(() => decodeStatus(status)).toThrow("status.state");
	});

	test.each([["a string", "12"], ["a fraction", 1.5], ["a negative number", -1], ["null", null]])(
		"is refused when a count is %s",
		(_, value) =>
		{
			const status = scanning();
			status.scan.groups = value;

			expect(() => decodeStatus(status)).toThrow("status.scan.groups");
		},
	);

	test("is refused when a reason count names an unknown reason", () =>
	{
		const status = scanning();
		status.scan.skippedByReason = { gremlins: 3 };

		expect(() => decodeStatus(status)).toThrow("status.scan.skippedByReason.gremlins");
	});

	test("is refused when it is not an object at all", () =>
	{
		expect(() => decodeStatus("scanning")).toThrow(DecodeError);
		expect(() => decodeGroupPage(null)).toThrow(DecodeError);
		expect(() => decodeSkipPage([])).toThrow(DecodeError);
	});

	test("names the row and the field inside a page", () =>
	{
		const page = structuredClone(contract("groups-page.json")) as { groups: { files: Record<string, unknown>[] }[] };
		const file = page.groups[1]?.files[0];
		if (file === undefined)
		{
			throw new Error("the example lost its second group");
		}
		file.modifiedMs = "yesterday";

		expect(() => decodeGroupPage(page)).toThrow("groups.groups[1].files[0].modifiedMs");
	});
});

describe("a refusal that is not one of ours", () =>
{
	test("keeps what was said", () =>
	{
		// Tauri answers with plain text when it cannot find a command or
		// read an argument.
		const error = decodeError("Command scan_stat not found");

		expect(error.code).toBe("unexpected");
		expect(error.message).toContain("Command scan_stat not found");
	});

	test("keeps an error thrown on this side", () =>
	{
		const error = decodeError(new DecodeError("status.state: expected one of the states"));

		expect(error.code).toBe("unexpected");
		expect(error.message).toContain("status.state");
	});

	test("does not take an unknown code for a known one", () =>
	{
		const error = decodeError({ code: "outOfCoffee" });

		expect(error.code).toBe("unexpected");
		expect(error.message).toContain("outOfCoffee");
	});
});
