import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import type { Backend } from "./backend/backend";
import { FakeBackend, withOverrides } from "./backend/fake";
import type { Status } from "./backend/types";
import { BackendError } from "./backend/types";
import type { ScanHandle } from "./useScan";
import { POLL_MS, useScan } from "./useScan";

const options = { includeHidden: false };

beforeEach(() =>
{
	vi.useFakeTimers();
});

afterEach(() =>
{
	vi.useRealTimers();
});

/** Lets `ms` of time pass and every answer that is due arrive. */
async function pass(ms = 0): Promise<void>
{
	await act(async () =>
	{
		await vi.advanceTimersByTimeAsync(ms);
	});
}

/** Does what a user or the backend does, the way React wants it done in a test. */
function order(action: () => void): void
{
	act(() =>
	{
		action();
	});
}

function begin(result: { current: ScanHandle }): void
{
	order(() =>
	{
		result.current.start(["/a"], options);
	});
}

/** Lets a held status call be answered: a scan is running. */
function answer(held: ((status: Status) => void)[], index: number): void
{
	order(() =>
	{
		held[index]?.(scanning(1));
	});
}

function statusCalls(fake: FakeBackend): number
{
	return fake.calls.filter(call => call === "status").length;
}

/**
 * A backend whose status answers only when the test says so, to put answers
 * in an order of the test's choosing. Everything else goes to a fake.
 */
function withHeldStatus(fake: FakeBackend): { backend: Backend; held: ((status: Status) => void)[] }
{
	const held: ((status: Status) => void)[] = [];
	const backend = withOverrides(fake, {
		status: () => new Promise<Status>((resolve) =>
		{
			held.push(resolve);
		}),
	});

	return { backend, held };
}

function scanning(id: number): Status
{
	return {
		state: "scanning",
		scan: {
			id,
			roots: ["/a"],
			phase: "walking",
			filesSeen: 0,
			filesDone: 0,
			filesTotal: 0,
			bytesDone: 0,
			bytesTotal: 0,
			groups: 0,
			reclaimableBytes: 0,
			skipped: 0,
			skippedByReason: {},
			summary: null,
			error: null,
		},
	};
}

test("it asks for the status once and does not poll while nothing runs", async () =>
{
	const fake = new FakeBackend();
	const { result } = renderHook(() => useScan(fake));

	await pass();
	expect(result.current.status.state).toBe("idle");
	expect(fake.calls).toEqual(["status"]);

	await pass(10 * POLL_MS);
	expect(fake.calls).toEqual(["status"]);
});

test("it polls while a scan runs and stops when the scan ends", async () =>
{
	const fake = new FakeBackend();
	const { result } = renderHook(() => useScan(fake));
	await pass();

	begin(result);
	await pass();
	expect(result.current.status.state).toBe("scanning");
	const before = statusCalls(fake);

	fake.report({ filesSeen: 7 });
	await pass(POLL_MS);
	expect(statusCalls(fake)).toBe(before + 1);
	expect(result.current.status.scan?.filesSeen).toBe(7);

	fake.end("completed");
	await pass(POLL_MS);
	expect(result.current.status.state).toBe("completed");
	const atTheEnd = statusCalls(fake);

	await pass(10 * POLL_MS);
	expect(statusCalls(fake)).toBe(atTheEnd);
});

test("it keeps polling while the scan is paused or stopping", async () =>
{
	const fake = new FakeBackend();
	const { result } = renderHook(() => useScan(fake));
	await pass();
	begin(result);
	await pass();

	order(result.current.pause);
	await pass();
	expect(result.current.status.state).toBe("paused");
	const whilePaused = statusCalls(fake);
	await pass(3 * POLL_MS);
	expect(statusCalls(fake)).toBe(whilePaused + 3);

	order(result.current.cancel);
	await pass();
	expect(result.current.status.state).toBe("stopping");
	fake.end("cancelled");
	await pass(POLL_MS);
	expect(result.current.status.state).toBe("cancelled");
});

test("it picks up a scan that was already running when the window opened", async () =>
{
	const fake = new FakeBackend();
	await fake.start(["/a"], options);

	const { result } = renderHook(() => useScan(fake));
	await pass();
	expect(result.current.status.state).toBe("scanning");

	const before = statusCalls(fake);
	await pass(2 * POLL_MS);
	expect(statusCalls(fake)).toBe(before + 2);
});

test("it never has two status calls on the way at once", async () =>
{
	const fake = new FakeBackend();
	const { backend, held } = withHeldStatus(fake);
	renderHook(() => useScan(backend));
	await pass();
	expect(held).toHaveLength(1);

	// The first answer says a scan runs, so polling begins.
	answer(held, 0);
	await pass();
	await pass(POLL_MS);
	expect(held).toHaveLength(2);

	// The backend is slow: a long time passes with no answer, and no
	// second question is piled on top of the first.
	await pass(20 * POLL_MS);
	expect(held).toHaveLength(2);

	answer(held, 1);
	await pass();
	await pass(POLL_MS);
	expect(held).toHaveLength(3);
});

test("an answer that arrives late does not undo a newer one", async () =>
{
	const fake = new FakeBackend();
	await fake.start(["/a"], options);
	const { backend, held } = withHeldStatus(fake);
	const { result } = renderHook(() => useScan(backend));
	await pass();

	// The status asked for at the start is still on its way when the user
	// pauses, and the pause is answered first.
	order(result.current.pause);
	await pass();
	expect(result.current.status.state).toBe("paused");

	answer(held, 0);
	await pass();
	expect(result.current.status.state).toBe("paused");
});

test("a call that fails is reported, and the next answer clears the report", async () =>
{
	const fake = new FakeBackend();
	const { result } = renderHook(() => useScan(fake));
	await pass();

	fake.failNext(new BackendError("unexpected", "the window lost its backend"));
	begin(result);
	await pass();
	expect(result.current.error?.message).toBe("the window lost its backend");
	expect(result.current.status.state).toBe("idle");

	begin(result);
	await pass();
	expect(result.current.error).toBeNull();
	expect(result.current.status.state).toBe("scanning");
});

test("a status call that fails does not stop the polling", async () =>
{
	const fake = new FakeBackend();
	const { result } = renderHook(() => useScan(fake));
	await pass();
	begin(result);
	await pass();

	fake.failNext(new Error("a hiccup"));
	await pass(POLL_MS);
	expect(result.current.error?.message).toBe("a hiccup");
	expect(result.current.status.state).toBe("scanning");

	fake.report({ filesSeen: 3 });
	await pass(POLL_MS);
	expect(result.current.error).toBeNull();
	expect(result.current.status.scan?.filesSeen).toBe(3);
});

test("it stops asking once the window is gone", async () =>
{
	const fake = new FakeBackend();
	await fake.start(["/a"], options);
	const { unmount } = renderHook(() => useScan(fake));
	await pass(POLL_MS);

	unmount();
	const after = statusCalls(fake);
	await pass(10 * POLL_MS);

	expect(statusCalls(fake)).toBe(after);
});
