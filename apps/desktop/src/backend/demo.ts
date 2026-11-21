import type { Backend } from "./backend";
import { FakeBackend, sampleGroup, withOverrides } from "./fake";

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
