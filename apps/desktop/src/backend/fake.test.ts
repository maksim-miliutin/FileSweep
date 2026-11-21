import { expect, test } from "vitest";

import { decodeGroupPage, decodeSkipPage, decodeStatus } from "./decode";
import { FakeBackend, sampleGroup, withOverrides } from "./fake";
import { BackendError } from "./types";

const options = { includeHidden: false };

async function refusalOf(call: Promise<unknown>): Promise<unknown>
{
	return call.then(() => "it was not refused", (error: unknown) => error);
}

test("a fake scan goes from idle to scanning to completed", async () =>
{
	const fake = new FakeBackend();
	expect(await fake.status()).toEqual({ state: "idle", scan: null });

	const started = await fake.start(["/a", "/b"], options);
	expect(started.state).toBe("scanning");
	expect(started.scan).toMatchObject({ id: 1, roots: ["/a", "/b"], phase: "walking", groups: 0 });
	expect(fake.startedWith).toEqual(options);

	fake.report({ phase: "hashing", filesSeen: 10, filesDone: 2, filesTotal: 4, bytesDone: 20, bytesTotal: 40 });
	fake.find(sampleGroup(100, 100));
	fake.skip({ path: "/a/locked", reason: "accessDenied", detail: null });
	expect((await fake.status()).scan).toMatchObject({
		phase: "hashing",
		filesSeen: 10,
		groups: 1,
		reclaimableBytes: 100,
		skipped: 1,
		skippedByReason: { accessDenied: 1 },
	});

	fake.end("completed");
	const finished = await fake.status();
	expect(finished.state).toBe("completed");
	expect(finished.scan?.summary).toMatchObject({ groups: 1, reclaimableBytes: 100, skipped: 1 });
});

test("a start is refused the way the session refuses it", async () =>
{
	const fake = new FakeBackend();

	expect(await refusalOf(fake.start([], options))).toMatchObject({ code: "noRoots" });

	await fake.start(["/a"], options);
	const busy = await refusalOf(fake.start(["/b"], options));
	expect(busy).toBeInstanceOf(BackendError);
	expect(busy).toMatchObject({ code: "busy" });

	fake.end("completed");
	expect((await fake.start(["/b"], options)).scan?.id).toBe(2);
});

test("pause, resume and cancel follow the states of the session", async () =>
{
	const fake = new FakeBackend();
	expect((await fake.pause()).state).toBe("idle");

	await fake.start(["/a"], options);
	expect((await fake.pause()).state).toBe("paused");
	expect((await fake.resume()).state).toBe("scanning");
	expect((await fake.cancel()).state).toBe("stopping");
	// A stopping scan takes no more orders.
	expect((await fake.resume()).state).toBe("stopping");
	expect((await fake.pause()).state).toBe("stopping");

	fake.end("cancelled");
	expect((await fake.status()).state).toBe("cancelled");
	expect((await fake.cancel()).state).toBe("cancelled");
});

test("a failed scan carries what went wrong", async () =>
{
	const fake = new FakeBackend();
	await fake.start(["/a"], options);

	fake.end("failed", "the scanner stopped unexpectedly");

	expect(await fake.status()).toMatchObject({
		state: "failed",
		scan: { error: "the scanner stopped unexpectedly", summary: null },
	});
});

test("pages carry the total and keep to the offset, the limit and the order", async () =>
{
	const fake = new FakeBackend();
	const { scan } = await fake.start(["/a"], options);
	const id = scan?.id ?? 0;
	for (const [size, saving] of [[90, 30], [80, 0], [70, 70], [60, 30], [50, 70]] as const)
	{
		fake.find(sampleGroup(size, saving));
	}

	const found = await fake.groups(id, { offset: 1, limit: 2, sort: "found" });
	expect(found).toMatchObject({ total: 5, offset: 1 });
	expect(found.groups.map(group => group.id)).toEqual([1, 2]);

	const bySavings = await fake.groups(id, { offset: 0, limit: 10, sort: "savings" });
	expect(bySavings.groups.map(group => group.id)).toEqual([2, 4, 0, 3, 1]);

	expect((await fake.groups(id, { offset: 9, limit: 10, sort: "found" })).groups).toEqual([]);

	for (let i = 0; i < 3; i += 1)
	{
		fake.skip({ path: `/a/${String(i)}`, reason: "hidden", detail: null });
	}
	const skipped = await fake.skipped(id, { offset: 2, limit: 5 });
	expect(skipped).toEqual({ total: 3, offset: 2, items: [{ path: "/a/2", reason: "hidden", detail: null }] });
});

test("a query for another scan is refused as stale", async () =>
{
	const fake = new FakeBackend();
	const query = { offset: 0, limit: 10 };

	expect(await refusalOf(fake.groups(1, { ...query, sort: "found" }))).toMatchObject({ code: "staleScan" });

	await fake.start(["/a"], options);
	expect(await refusalOf(fake.skipped(2, query))).toMatchObject({ code: "staleScan" });
});

test("what the fake answers has the shape the real one is held to", async () =>
{
	const fake = new FakeBackend();
	const { scan } = await fake.start(["/a"], options);
	const id = scan?.id ?? 0;
	fake.find(sampleGroup(10, 10));
	fake.skip({ path: "/a/broken", reason: "io", detail: "the device is not ready" });
	fake.end("completed");

	const status = await fake.status();
	const groups = await fake.groups(id, { offset: 0, limit: 10, sort: "found" });
	const skipped = await fake.skipped(id, { offset: 0, limit: 10 });

	expect(decodeStatus(status)).toEqual(status);
	expect(decodeGroupPage(groups)).toEqual(groups);
	expect(decodeSkipPage(skipped)).toEqual(skipped);
});

test("a test can make the next call fail and can see what was called", async () =>
{
	const fake = new FakeBackend();
	fake.failNext(new BackendError("unexpected", "the window lost its backend"));

	expect(await refusalOf(fake.status())).toMatchObject({ code: "unexpected" });
	expect((await fake.status()).state).toBe("idle");
	expect(fake.calls).toEqual(["status", "status"]);
});

test("its state can be read without a call", async () =>
{
	const fake = new FakeBackend();
	expect(fake.state).toBe("idle");

	await fake.start(["/a"], options);
	await fake.pause();
	expect(fake.state).toBe("paused");
	expect(fake.calls).toEqual(["start", "pause"]);
});

test("the folders it offers are the ones the test chose", async () =>
{
	const fake = new FakeBackend();
	fake.picks = ["/home/me/Downloads"];

	expect(await fake.pickFolders()).toEqual(["/home/me/Downloads"]);
});

test("one answer can be replaced while the rest stay the fake's", async () =>
{
	const fake = new FakeBackend();
	const backend = withOverrides(fake, { pickFolders: () => Promise.resolve(["/chosen"]) });

	expect(await backend.pickFolders()).toEqual(["/chosen"]);
	expect((await backend.start(["/a"], options)).state).toBe("scanning");
	expect((await backend.pause()).state).toBe("paused");
	expect(fake.calls).toEqual(["start", "pause"]);
});

test("the folders it suggests are the ones the test gave it", async () =>
{
	const fake = new FakeBackend();
	expect(await fake.suggestedRoots()).toEqual([]);

	fake.suggestions = [{ kind: "downloads", path: "/home/me/Downloads" }];

	expect(await fake.suggestedRoots()).toEqual([{ kind: "downloads", path: "/home/me/Downloads" }]);
});
