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

test("only the first rows are shown, and the page says how many there are", async () =>
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

	const groups = screen.getByRole("list", { name: "Groups of identical files" });
	expect(groups.children).toHaveLength(50);
	expect(screen.getByText("Showing the first 50 of 60 groups.")).toBeTruthy();
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
