import type { Backend } from "./backend";
import { FakeBackend, sampleFile, sampleGroup, withOverrides } from "./fake";

const STEP_MS = 700;

/**
 * For looking at the interface in a plain browser, where there is no shell:
 * the development server uses it, and no build for release includes it.
 */
export function demoBackend(): Backend
{
	const fake = new FakeBackend();
	fake.picks = ["D:\\Archive\\2019", "D:\\Archive\\Camera"];
	fake.suggestions = [
		{ kind: "downloads", path: "C:\\Users\\me\\Downloads" },
		{ kind: "desktop", path: "C:\\Users\\me\\Desktop" },
		{ kind: "documents", path: "C:\\Users\\me\\OneDrive\\Documents" },
		{ kind: "pictures", path: "C:\\Users\\me\\Pictures" },
	];

	return withOverrides(fake, {
		start: async (roots, options) =>
		{
			const status = await fake.start(roots, options);
			play(fake);

			return status;
		},
	});
}

function play(fake: FakeBackend): void
{
	const megabyte = 1024 * 1024;
	const steps: (() => void)[] = [
		() =>
		{
			fake.report({ filesSeen: 420 });
		},
		() =>
		{
			fake.report({ filesSeen: 1260 });
			fake.skip({ path: "C:\\Users\\me\\Downloads\\AppData", reason: "hidden", detail: null });
			indexSomeYears(fake, megabyte);
		},
		() =>
		{
			fake.report({ phase: "hashing", filesTotal: 300, bytesTotal: 700 * megabyte });
		},
		() =>
		{
			fake.report({ filesDone: 90, bytesDone: 310 * megabyte });
			fake.find(sampleGroup(120 * megabyte, 240 * megabyte));
		},
		() =>
		{
			fake.report({ filesDone: 210, bytesDone: 590 * megabyte });
			fake.find(sampleGroup(18 * megabyte, 18 * megabyte));
			fake.skip({ path: "C:\\Users\\me\\Pictures\\locked.db", reason: "inUse", detail: null });
		},
		() =>
		{
			fake.report({ filesDone: 300, bytesDone: 700 * megabyte });
			fake.find({ ...sampleGroup(4096, 0), independence: "unverified" });
			fake.end("completed");
		},
	];

	let next = 0;
	const timer = setInterval(() =>
	{
		if (fake.state === "paused")
		{
			return;
		}

		if (fake.state === "stopping")
		{
			fake.end("cancelled");
		}

		if (fake.state !== "scanning")
		{
			clearInterval(timer);

			return;
		}

		steps[next]?.();
		next += 1;
	}, STEP_MS);
}

/** Files from the years a real folder of downloads gathers, for the usage view. */
function indexSomeYears(fake: FakeBackend, megabyte: number): void
{
	const midYear = (year: number) => Date.UTC(year, 5, 15);
	const files: [string, number, number][] = [
		["C:\\Users\\me\\Downloads\\ubuntu-16.04-desktop-amd64.iso", 1500, 2016],
		["C:\\Users\\me\\Downloads\\Setup_Office_2016.exe", 900, 2017],
		["C:\\Users\\me\\Downloads\\wedding-video-raw.mov", 6200, 2018],
		["C:\\Users\\me\\Documents\\thesis-backup.zip", 340, 2019],
		["C:\\Users\\me\\Downloads\\game-patch-1.4.zip", 2100, 2020],
		["C:\\Users\\me\\Pictures\\Camera\\IMG_2041.CR2", 28, 2021],
		["C:\\Users\\me\\Downloads\\driver-pack-2022.exe", 610, 2022],
		["C:\\Users\\me\\Documents\\taxes-2023.pdf", 2, 2023],
		["C:\\Users\\me\\Downloads\\node-v20-x64.msi", 31, 2024],
		["C:\\Users\\me\\Downloads\\Screen Recording.mp4", 450, 2025],
	];

	for (const [path, megabytes, year] of files)
	{
		fake.index(sampleFile(path, megabytes * megabyte, midYear(year)));
	}

	fake.index({ path: "C:\\Users\\me\\Downloads\\no-date.bin", size: megabyte, modifiedMs: null, accessedMs: null, createdMs: null });
}
