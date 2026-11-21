import { expect, test } from "vitest";

import { contract, contractObject } from "../test-contract";
import { DecodeError } from "./decode";
import type { Invoke } from "./tauri";
import { tauriBackend } from "./tauri";
import type { Backend } from "./backend";
import { BackendError } from "./types";

interface Call
{
	command: string;
	payload: unknown;
}

/** A backend whose invoke answers with `answer` and writes down what it was asked. */
function recording(answer: unknown): { backend: Backend; calls: Call[] }
{
	const calls: Call[] = [];
	const invoke: Invoke = (command, payload) =>
	{
		calls.push({ command, payload });

		return Promise.resolve(answer);
	};

	return { backend: tauriBackend(invoke, () => Promise.resolve([])), calls };
}

function refusing(refusal: unknown): Backend
{
	// Tauri rejects with what the command returned, an object or plain text,
	// and never with an Error. Standing in for it means doing the same.
	// eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors
	return tauriBackend(() => Promise.reject(refusal), () => Promise.resolve([]));
}

// The Rust side is tested against the same file: it calls every example
// through Tauri's IPC and expects none to be turned down. A name or an
// argument that changes on one side only goes red here or there.
test("every call sends the command and the arguments the contract lists", async () =>
{
	const examples = contractObject("commands.json");
	const status = contract("status-idle.json");
	const groups = contract("groups-page.json");
	const skipped = contract("skipped-page.json");

	const ask: Record<string, { answer: unknown; call: (backend: Backend) => Promise<unknown> }> = {
		start: {
			answer: status,
			call: backend => backend.start(["/nonexistent/filesweep-example"], { includeHidden: false }),
		},
		pause: { answer: status, call: backend => backend.pause() },
		resume: { answer: status, call: backend => backend.resume() },
		cancel: { answer: status, call: backend => backend.cancel() },
		status: { answer: status, call: backend => backend.status() },
		groups: {
			answer: groups,
			call: backend => backend.groups(1, { offset: 0, limit: 50, sort: "savings" }),
		},
		skipped: {
			answer: skipped,
			call: backend => backend.skipped(1, { offset: 0, limit: 200 }),
		},
		suggestedRoots: {
			answer: contract("suggested-roots.json"),
			call: backend => backend.suggestedRoots(),
		},
		reveal: { answer: null, call: backend => backend.reveal(1, 2, 3) },
	};

	expect(Object.keys(ask).sort()).toEqual(Object.keys(examples).sort());
	for (const [name, { answer, call }] of Object.entries(ask))
	{
		const { backend, calls } = recording(answer);

		await call(backend);

		expect(calls, name).toEqual([examples[name]]);
	}
});

// The contract's example asks for no hidden files, so a backend that always
// sent `false` would pass the test above.
test("the choice about hidden files is sent as it was made", async () =>
{
	const { backend, calls } = recording(contract("status-idle.json"));

	await backend.start(["/a"], { includeHidden: true });

	expect(calls).toEqual([{ command: "scan_start", payload: { roots: ["/a"], includeHidden: true } }]);
});

test("showing a file answers with nothing, and an answer with something in it is refused", async () =>
{
	await expect(recording(null).backend.reveal(1, 2, 3)).resolves.toBeUndefined();
	await expect(recording({ shown: true }).backend.reveal(1, 2, 3)).rejects.toBeInstanceOf(DecodeError);
});

test("an answer comes back decoded", async () =>
{
	const { backend } = recording(contract("status-scanning.json"));

	await expect(backend.status()).resolves.toEqual(contract("status-scanning.json"));
});

test("an answer of the wrong shape is refused instead of passed on", async () =>
{
	const { backend } = recording({ state: "scanning" });

	await expect(backend.status()).rejects.toBeInstanceOf(DecodeError);
});

test("a coded refusal becomes a backend error", async () =>
{
	const backend = refusing({ code: "tooManyRoots", max: 64 });

	const refusal: unknown = await backend.start(["/a"], { includeHidden: false }).catch((error: unknown) => error);

	expect(refusal).toBeInstanceOf(BackendError);
	expect(refusal).toMatchObject({ code: "tooManyRoots", detail: { max: 64 } });
});

test("a refusal by Tauri itself becomes an unexpected backend error", async () =>
{
	const backend = refusing("invalid args `scanId` for command `scan_groups`");

	const refusal: unknown = await backend.groups(1, { offset: 0, limit: 1, sort: "found" }).catch((error: unknown) => error);

	expect(refusal).toMatchObject({ code: "unexpected" });
});

test("picking folders is handed to the dialog", async () =>
{
	const backend = tauriBackend(() => Promise.resolve(null), () => Promise.resolve(["/a", "/b"]));

	await expect(backend.pickFolders()).resolves.toEqual(["/a", "/b"]);
});
