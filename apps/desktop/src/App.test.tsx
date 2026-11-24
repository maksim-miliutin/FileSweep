import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { App } from "./App";
import type { Backend } from "./backend/backend";
import { FakeBackend, sampleGroup, withOverrides } from "./backend/fake";
import { BackendError } from "./backend/types";
import { POLL_MS } from "./useScan";

beforeEach(() =>
{
	vi.useFakeTimers();
});

afterEach(() =>
{
	vi.useRealTimers();
});

async function pass(ms = 0): Promise<void>
{
	await act(async () =>
	{
		await vi.advanceTimersByTimeAsync(ms);
	});
}

function button(name: string): HTMLButtonElement
{
	return screen.getByRole<HTMLButtonElement>("button", { name });
}

function hasButton(name: string): boolean
{
	return screen.queryByRole("button", { name }) !== null;
}

function chosenFolders(): (string | null | undefined)[]
{
	return within(screen.getByRole("list", { name: "Folders to scan" }))
		.getAllByRole("listitem")
		.map(item => item.firstChild?.textContent);
}

/** The window with the given folders already chosen through the dialog. */
async function opened(fake: FakeBackend, folders: string[] = [], backend: Backend = fake): Promise<void>
{
	render(<App backend={backend} />);
	await pass();

	if (folders.length > 0)
	{
		fake.picks = folders;
		fireEvent.click(button("Add folder"));
		await pass();
	}
}

async function scanStarted(fake: FakeBackend): Promise<void>
{
	await opened(fake, ["/home/me/Downloads"]);
	fireEvent.click(button("Scan"));
	await pass();
}

test("the window names the application", async () =>
{
	await opened(new FakeBackend());

	expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("FileSweep");
});

test("folders come from the dialog, once each, and can be taken off the list", async () =>
{
	const fake = new FakeBackend();
	await opened(fake);
	expect(screen.getByText("No folder chosen yet.")).toBeTruthy();

	fake.picks = ["/home/me/Downloads", "/home/me/Pictures"];
	fireEvent.click(button("Add folder"));
	await pass();
	fake.picks = ["/home/me/Pictures", "/home/me/Music"];
	fireEvent.click(button("Add folder"));
	await pass();

	const chosen = within(screen.getByRole("list", { name: "Folders to scan" })).getAllByRole("listitem");
	expect(chosen.map(item => item.firstChild?.textContent)).toEqual([
		"/home/me/Downloads",
		"/home/me/Pictures",
		"/home/me/Music",
	]);

	fireEvent.click(button("Remove /home/me/Pictures"));
	expect(screen.queryByText("/home/me/Pictures")).toBeNull();
	expect(screen.getByText("/home/me/Music")).toBeTruthy();
});

test("a window opened under a scan shows the folders it scans, ready to scan again", async () =>
{
	const fake = new FakeBackend();
	await fake.start(["/home/me/Downloads", "/home/me/Pictures"], { includeHidden: false });
	await opened(fake);

	expect(chosenFolders()).toEqual(["/home/me/Downloads", "/home/me/Pictures"]);

	fake.end("completed");
	await pass(POLL_MS);
	fireEvent.click(button("Scan"));
	await pass();

	const status = await fake.status();
	expect(status.scan?.id).toBe(2);
	expect(status.scan?.roots).toEqual(["/home/me/Downloads", "/home/me/Pictures"]);
});

test("folders taken off the list are not put back from the last scan", async () =>
{
	const fake = new FakeBackend();
	await fake.start(["/a", "/b"], { includeHidden: false });
	fake.end("completed");
	await opened(fake);

	fireEvent.click(button("Remove /a"));
	fireEvent.click(button("Remove /b"));
	await pass();

	expect(screen.getByText("No folder chosen yet.")).toBeTruthy();
	expect(button("Scan").disabled).toBe(true);
});

test("a dialog the user closes without choosing changes nothing", async () =>
{
	const fake = new FakeBackend();
	await opened(fake, ["/a"]);

	fake.picks = [];
	fireEvent.click(button("Add folder"));
	await pass();

	expect(screen.getAllByRole("listitem")).toHaveLength(1);
});

test("a scan is offered only when there is a folder to scan", async () =>
{
	const fake = new FakeBackend();
	await opened(fake);
	expect(button("Scan").disabled).toBe(true);

	fake.picks = ["/a"];
	fireEvent.click(button("Add folder"));
	await pass();

	expect(button("Scan").disabled).toBe(false);
});

test("starting sends the chosen folders and the choice about hidden files", async () =>
{
	const fake = new FakeBackend();
	await opened(fake, ["/a", "/b"]);

	fireEvent.click(screen.getByRole("checkbox", { name: "Include hidden files" }));
	fireEvent.click(button("Scan"));
	await pass();

	expect(fake.startedWith).toEqual({ includeHidden: true });
	const status = await fake.status();
	expect(status.scan?.roots).toEqual(["/a", "/b"]);
	expect(screen.getByRole("status").textContent).toContain("Looking for files");
});

test("the controls offered are the ones the state allows", async () =>
{
	const fake = new FakeBackend();
	await opened(fake, ["/a"]);
	expect([hasButton("Pause"), hasButton("Resume"), hasButton("Cancel")]).toEqual([false, false, false]);

	fireEvent.click(button("Scan"));
	await pass();
	expect([hasButton("Pause"), hasButton("Resume"), hasButton("Cancel")]).toEqual([true, false, true]);
	// The folders of a running scan are not to be changed under it.
	expect(button("Scan").disabled).toBe(true);
	expect(button("Add folder").disabled).toBe(true);
	expect(button("Remove /a").disabled).toBe(true);

	fireEvent.click(button("Pause"));
	await pass();
	expect([hasButton("Pause"), hasButton("Resume"), hasButton("Cancel")]).toEqual([false, true, true]);
	expect(screen.getByRole("status").textContent).toContain("Paused");

	fireEvent.click(button("Resume"));
	await pass();
	expect([hasButton("Pause"), hasButton("Resume"), hasButton("Cancel")]).toEqual([true, false, true]);

	fireEvent.click(button("Cancel"));
	await pass();
	expect([hasButton("Pause"), hasButton("Resume"), hasButton("Cancel")]).toEqual([false, false, false]);
	expect(screen.getByRole("status").textContent).toContain("Stopping");
	expect(button("Scan").disabled).toBe(true);

	fake.end("cancelled");
	await pass(POLL_MS);
	expect(screen.getByRole("status").textContent).toContain("Cancelled");
	expect(button("Scan").disabled).toBe(false);
});

test("progress is told in files and in sizes", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);

	fake.report({ filesSeen: 1200 });
	await pass(POLL_MS);
	expect(screen.getByRole("status").textContent).toContain("1,200");

	fake.report({ phase: "hashing", filesDone: 40, filesTotal: 300, bytesDone: 1_048_576, bytesTotal: 734_003_200 });
	await pass(POLL_MS);
	const status = screen.getByRole("status").textContent;
	expect(status).toContain("40 of 300");
	expect(status).toContain("1 MB of 700 MB");
});

test("a finished scan lists its groups, the most reclaimable first", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);
	fake.find(sampleGroup(2048, 2048));
	fake.find(sampleGroup(1024, 5120));
	fake.end("completed");
	await pass(POLL_MS);
	await pass();

	expect(screen.getByRole("status").textContent).toContain("2 groups");
	expect(screen.getByRole("status").textContent).toContain("7 KB");

	const groups = within(screen.getByRole("list", { name: "Groups of identical files" })).getAllByRole("listitem")
		.filter(item => item.parentElement?.getAttribute("aria-label") === "Groups of identical files");
	expect(groups).toHaveLength(2);
	expect(groups[0]?.textContent).toContain("5 KB can be freed");
	expect(groups[0]?.textContent).toContain("/sample/1024/one.bin");
	expect(groups[0]?.textContent).toContain("/sample/1024/two.bin");
	expect(groups[1]?.textContent).toContain("2 KB can be freed");
});

test("groups are shown while the scan still runs, in the order they are found", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);

	fake.find(sampleGroup(2048, 2048));
	fake.find(sampleGroup(1024, 5120));
	await pass(POLL_MS);
	await pass();

	const groups = screen.getByRole("list", { name: "Groups of identical files" });
	expect(groups.firstElementChild?.textContent).toContain("/sample/2048/one.bin");
});

test("a group that promises nothing says why", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);
	fake.find({ ...sampleGroup(1024, 0), independence: "unverified" });
	fake.end("completed");
	await pass(POLL_MS);
	await pass();

	const groups = screen.getByRole("list", { name: "Groups of identical files" });
	expect(groups.textContent).toContain("may be one file on disk");
});

test("skipped entries are listed with the reason for each", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);
	fake.skip({ path: "/a/locked", reason: "accessDenied", detail: null });
	fake.skip({ path: "/a/broken", reason: "io", detail: "the device is not ready" });
	fake.end("completed");
	await pass(POLL_MS);
	await pass();

	const skipped = within(screen.getByRole("list", { name: "Skipped entries" })).getAllByRole("listitem");
	expect(skipped).toHaveLength(2);
	expect(skipped[0]?.textContent).toContain("/a/locked");
	expect(skipped[0]?.textContent).toContain("access was denied");
	expect(skipped[1]?.textContent).toContain("the device is not ready");
});

test("a failed scan says what went wrong", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);

	fake.end("failed", "the scanner stopped unexpectedly: boom");
	await pass(POLL_MS);

	expect(screen.getByRole("status").textContent).toContain("the scanner stopped unexpectedly: boom");
	expect(button("Scan").disabled).toBe(false);
});

test("a refusal is shown, and the page goes on working", async () =>
{
	const fake = new FakeBackend();
	await opened(fake, ["/a"]);

	fake.failNext(new BackendError("relativeRoot", "a folder path is not absolute", { path: "Downloads" }));
	fireEvent.click(button("Scan"));
	await pass();
	expect(screen.getByRole("alert").textContent).toContain("a folder path is not absolute");

	fireEvent.click(button("Scan"));
	await pass();
	expect(screen.queryByRole("alert")).toBeNull();
	expect(hasButton("Pause")).toBe(true);
});

test("a dialog that cannot be opened is reported", async () =>
{
	const fake = new FakeBackend();
	await opened(fake);

	fake.failNext(new Error("the dialog is not allowed"));
	fireEvent.click(button("Add folder"));
	await pass();

	expect(screen.getByRole("alert").textContent).toContain("the dialog is not allowed");
});

test("a new scan clears the rows of the one before it", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);
	fake.find(sampleGroup(2048, 2048));
	fake.end("completed");
	await pass(POLL_MS);
	await pass();
	expect(screen.getByRole("list", { name: "Groups of identical files" }).children).toHaveLength(1);

	fireEvent.click(button("Scan"));
	await pass();
	await pass();

	expect(screen.queryByRole("list", { name: "Groups of identical files" })).toBeNull();
});

test("rows refused because a newer scan took over are not told as a failure", async () =>
{
	const fake = new FakeBackend();
	const stale = new BackendError("staleScan", "the results belong to an earlier scan");
	await opened(fake, ["/a"], withOverrides(fake, { groups: () => Promise.reject(stale) }));
	fireEvent.click(button("Scan"));
	await pass();

	fake.find(sampleGroup(2048, 2048));
	await pass(POLL_MS);
	await pass();

	expect(screen.queryByRole("alert")).toBeNull();
});

test("rows that cannot be read are told as a failure", async () =>
{
	const fake = new FakeBackend();
	const broken = new Error("groups.total: expected a whole number, got text");
	await opened(fake, ["/a"], withOverrides(fake, { groups: () => Promise.reject(broken) }));
	fireEvent.click(button("Scan"));
	await pass();

	fake.find(sampleGroup(2048, 2048));
	await pass(POLL_MS);
	await pass();

	expect(screen.getByRole("alert").textContent).toContain("groups.total");
});

test("known folders are offered until they are chosen, and one press adds one", async () =>
{
	const fake = new FakeBackend();
	fake.suggestions = [
		{ kind: "downloads", path: "/home/me/Downloads" },
		{ kind: "pictures", path: "/home/me/Pictures" },
	];
	await opened(fake);

	fireEvent.click(button("Add Downloads"));

	expect(chosenFolders()).toEqual(["/home/me/Downloads"]);
	expect([hasButton("Add Downloads"), hasButton("Add Pictures")]).toEqual([false, true]);
	expect(button("Scan").disabled).toBe(false);

	fireEvent.click(button("Remove /home/me/Downloads"));
	expect(hasButton("Add Downloads")).toBe(true);
});

test("a known folder is not added while a scan runs", async () =>
{
	const fake = new FakeBackend();
	fake.suggestions = [{ kind: "pictures", path: "/home/me/Pictures" }];
	await scanStarted(fake);

	expect(button("Add Pictures").disabled).toBe(true);
});

test("known folders that cannot be read are reported, and folders can still be added", async () =>
{
	const fake = new FakeBackend();
	const broken = withOverrides(fake, { suggestedRoots: () => Promise.reject(new Error("the shell did not answer")) });
	await opened(fake, [], broken);
	expect(screen.getByRole("alert").textContent).toContain("the shell did not answer");

	fake.picks = ["/a"];
	fireEvent.click(button("Add folder"));
	await pass();

	expect(chosenFolders()).toEqual(["/a"]);
	expect(screen.queryByRole("alert")).toBeNull();
});

function fileOf(place: number): HTMLElement
{
	const group = within(screen.getByRole("list", { name: "Groups of identical files" }))
		.getAllByRole("listitem")
		.find(item => item.parentElement?.getAttribute("aria-label") === "Groups of identical files");
	const file = group === undefined ? undefined : within(group).getAllByRole("listitem")[place];
	if (file === undefined)
	{
		throw new Error(`the first group has no file ${String(place)}`);
	}

	return file;
}

async function finishedWithOneGroup(fake: FakeBackend): Promise<void>
{
	await scanStarted(fake);
	fake.find(sampleGroup(1024, 1024));
	fake.end("completed");
	await pass(POLL_MS);
	await pass();
}

test("Show in folder names the file by its ids", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);
	fake.find(sampleGroup(1024, 1024));
	// Found second but frees more, so it is listed first: its id is 1, not its place.
	fake.find(sampleGroup(2048, 4096));
	fake.end("completed");
	await pass(POLL_MS);
	await pass();

	fireEvent.click(within(fileOf(1)).getByRole("button", { name: "Show in folder" }));
	await pass();

	expect(fake.revealed).toEqual([{ scanId: 1, groupId: 1, fileId: 1 }]);
});

test("a file that cannot be shown says why next to it, and the other buttons go on working", async () =>
{
	const fake = new FakeBackend();
	await finishedWithOneGroup(fake);

	fake.failNext(new BackendError("revealFailed", "the file could not be shown in its folder", { message: "path doesn't exist" }));
	fireEvent.click(within(fileOf(0)).getByRole("button", { name: "Show in folder" }));
	await pass();
	expect(within(fileOf(0)).getByRole("alert").textContent).toContain("path doesn't exist");

	fireEvent.click(within(fileOf(1)).getByRole("button", { name: "Show in folder" }));
	await pass();
	expect(fake.revealed).toEqual([{ scanId: 1, groupId: 0, fileId: 1 }]);
});

test("Copy path puts the whole path on the clipboard", async () =>
{
	const writeText = vi.fn(() => Promise.resolve());
	Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
	try
	{
		const fake = new FakeBackend();
		await finishedWithOneGroup(fake);

		fireEvent.click(within(fileOf(1)).getByRole("button", { name: "Copy path" }));
		await pass();

		expect(writeText).toHaveBeenCalledWith("/sample/1024/two.bin");
		expect(within(fileOf(1)).getByRole("status").textContent).toContain("Path copied");
	}
	finally
	{
		Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
	}
});

test("a clipboard the window cannot use is reported next to the button", async () =>
{
	const fake = new FakeBackend();
	await finishedWithOneGroup(fake);

	fireEvent.click(within(fileOf(0)).getByRole("button", { name: "Copy path" }));
	await pass();

	expect(within(fileOf(0)).getByRole("alert").textContent).toContain("clipboard");
});

function region(name: string): HTMLElement
{
	return screen.getByRole("region", { name });
}

function rowsIn(list: string): HTMLElement[]
{
	return within(screen.getByRole("list", { name: list })).getAllByRole("listitem")
		.filter(item => item.parentElement?.getAttribute("aria-label") === list);
}

function turn(where: string, name: "Previous page" | "Next page"): HTMLButtonElement
{
	return within(region(where)).getByRole<HTMLButtonElement>("button", { name });
}

test("the groups come in pages of 50, and the pages can be turned", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);
	for (let size = 60; size > 0; size -= 1)
	{
		fake.find(sampleGroup(size, size));
	}
	fake.end("completed");
	await pass(POLL_MS);
	await pass();

	expect(rowsIn("Groups of identical files")).toHaveLength(50);
	expect(region("Duplicates").textContent).toContain("Groups 1–50 of 60");
	expect([turn("Duplicates", "Previous page").disabled, turn("Duplicates", "Next page").disabled]).toEqual([true, false]);

	fireEvent.click(turn("Duplicates", "Next page"));
	await pass();
	expect(rowsIn("Groups of identical files")).toHaveLength(10);
	expect(region("Duplicates").textContent).toContain("Groups 51–60 of 60");
	expect(turn("Duplicates", "Next page").disabled).toBe(true);

	fireEvent.click(turn("Duplicates", "Previous page"));
	await pass();
	expect(rowsIn("Groups of identical files")).toHaveLength(50);
});

test("when the scan ends the list starts again from its first page", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);
	for (let size = 1; size <= 60; size += 1)
	{
		fake.find(sampleGroup(size, size));
	}
	await pass(POLL_MS);
	await pass();
	fireEvent.click(turn("Duplicates", "Next page"));
	await pass();
	expect(region("Duplicates").textContent).toContain("Groups 51–60 of 60");

	fake.end("completed");
	await pass(POLL_MS);
	await pass();

	expect(region("Duplicates").textContent).toContain("Groups 1–50 of 60");
	expect(rowsIn("Groups of identical files")[0]?.textContent).toContain("60 bytes can be freed");
});

test("the page shown is refreshed while the scan finds more", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);
	for (let size = 1; size <= 30; size += 1)
	{
		fake.find(sampleGroup(size, size));
	}
	await pass(POLL_MS);
	await pass();
	expect(region("Duplicates").textContent).toContain("Groups 1–30 of 30");

	for (let size = 31; size <= 60; size += 1)
	{
		fake.find(sampleGroup(size, size));
	}
	await pass(POLL_MS);
	await pass();

	expect(rowsIn("Groups of identical files")).toHaveLength(50);
	expect(region("Duplicates").textContent).toContain("Groups 1–50 of 60");
});

test("the skipped entries come in pages too, and are counted by reason", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);
	for (let place = 0; place < 55; place += 1)
	{
		fake.skip({ path: `/a/${String(place)}.lock`, reason: "inUse", detail: null });
	}
	for (const name of [".one", ".two", ".three"])
	{
		fake.skip({ path: `/a/${name}`, reason: "hidden", detail: null });
	}
	fake.skip({ path: "/a/broken", reason: "io", detail: "the device is not ready" });
	fake.skip({ path: "/a/bad", reason: "io", detail: "the device is not ready" });
	fake.end("completed");
	await pass(POLL_MS);
	await pass();

	expect(region("Skipped").textContent).toContain("Could not be read: in use 55, read errors 2.");
	expect(region("Skipped").textContent).toContain("Left out by rule: hidden 3.");
	const told = region("Skipped").textContent;
	expect(told.indexOf("Could not be read")).toBeLessThan(told.indexOf("Left out by rule"));
	expect(region("Skipped").textContent).toContain("Entries 1–50 of 60");

	fireEvent.click(turn("Skipped", "Next page"));
	await pass();
	expect(rowsIn("Skipped entries")).toHaveLength(10);
	expect(region("Skipped").textContent).toContain("Entries 51–60 of 60");
});

test("a list of exactly one full page has no next page", async () =>
{
	const fake = new FakeBackend();
	await scanStarted(fake);
	for (let size = 1; size <= 50; size += 1)
	{
		fake.find(sampleGroup(size, size));
	}
	fake.end("completed");
	await pass(POLL_MS);
	await pass();

	expect(turn("Duplicates", "Next page").disabled).toBe(true);
});
