import type { Backend, StartOptions } from "./backend";
import type {
	GroupPage,
	GroupQuery,
	GroupView,
	PageQuery,
	Phase,
	ScanState,
	ScanView,
	SkipPage,
	SkipReason,
	SkipView,
	Status,
} from "./types";
import { BackendError } from "./types";

/** The most rows a page carries, as in the session. */
const MAX_PAGE = 500;

interface Progress
{
	phase: Phase;
	filesSeen: number;
	filesDone: number;
	filesTotal: number;
	bytesDone: number;
	bytesTotal: number;
}

interface FakeScan
{
	id: number;
	roots: string[];
	state: Exclude<ScanState, "idle">;
	progress: Progress;
	groups: GroupView[];
	skipped: SkipView[];
	error: string | null;
}

/**
 * A backend in memory that keeps to the rules of the session: one scan at a
 * time, the same states, the same refusals. It scans nothing: a test tells it
 * what has been found and when the scan ends.
 */
export class FakeBackend implements Backend
{
	readonly calls: string[] = [];
	picks: string[] = [];

	/** The options of the last start that was accepted. */
	startedWith: StartOptions | null = null;

	private scan: FakeScan | null = null;
	private lastId = 0;
	private failure: Error | null = null;

	failNext(error: Error): void
	{
		this.failure = error;
	}

	report(progress: Partial<Progress>): void
	{
		const scan = this.running("report progress");
		scan.progress = { ...scan.progress, ...progress };
	}

	/** The group's id becomes its place in the order of finding. */
	find(group: Omit<GroupView, "id">): void
	{
		const scan = this.running("find a group");
		scan.groups.push({ ...group, id: scan.groups.length });
	}

	skip(entry: SkipView): void
	{
		this.running("skip an entry").skipped.push(entry);
	}

	end(state: "completed" | "cancelled" | "failed", error = "the scanner stopped unexpectedly"): void
	{
		const scan = this.running("end");
		scan.state = state;
		scan.error = state === "failed" ? error : null;
	}

	start(roots: readonly string[], options: StartOptions): Promise<Status>
	{
		return this.answer("start", () =>
		{
			if (roots.length === 0)
			{
				throw new BackendError("noRoots", "no folder was given to scan");
			}

			if (this.scan !== null && isRunning(this.scan))
			{
				throw new BackendError("busy", "a scan is already running");
			}

			this.lastId += 1;
			this.startedWith = { ...options };
			this.scan = {
				id: this.lastId,
				roots: [...roots],
				state: "scanning",
				progress: { phase: "walking", filesSeen: 0, filesDone: 0, filesTotal: 0, bytesDone: 0, bytesTotal: 0 },
				groups: [],
				skipped: [],
				error: null,
			};

			return this.snapshot();
		});
	}

	pause(): Promise<Status>
	{
		return this.order("pause", "scanning", "paused");
	}

	resume(): Promise<Status>
	{
		return this.order("resume", "paused", "scanning");
	}

	cancel(): Promise<Status>
	{
		return this.answer("cancel", () =>
		{
			if (this.scan !== null && (this.scan.state === "scanning" || this.scan.state === "paused"))
			{
				this.scan.state = "stopping";
			}

			return this.snapshot();
		});
	}

	status(): Promise<Status>
	{
		return this.answer("status", () => this.snapshot());
	}

	groups(scanId: number, query: GroupQuery): Promise<GroupPage>
	{
		return this.answer("groups", () =>
		{
			const scan = this.named(scanId);
			const ordered = query.sort === "found"
				? scan.groups
				// Array.sort is stable: equal savings stay in the order of finding.
				: [...scan.groups].sort((a, b) => b.reclaimableBytes - a.reclaimableBytes);

			return {
				total: scan.groups.length,
				offset: query.offset,
				groups: page(ordered, query),
			};
		});
	}

	skipped(scanId: number, query: PageQuery): Promise<SkipPage>
	{
		return this.answer("skipped", () =>
		{
			const scan = this.named(scanId);

			return { total: scan.skipped.length, offset: query.offset, items: page(scan.skipped, query) };
		});
	}

	pickFolders(): Promise<string[]>
	{
		return this.answer("pickFolders", () => [...this.picks]);
	}

	/** Every call goes through here: it is written down, and it can be made to fail. */
	private answer<T>(call: string, respond: () => T): Promise<T>
	{
		this.calls.push(call);

		const failure = this.failure;
		if (failure !== null)
		{
			this.failure = null;

			return Promise.reject(failure);
		}

		try
		{
			return Promise.resolve(respond());
		}
		catch (refusal)
		{
			return Promise.reject(refusal instanceof Error ? refusal : new Error(String(refusal)));
		}
	}

	/** An order that does not fit the state of the scan changes nothing, as in the session. */
	private order(call: string, from: ScanState, to: Exclude<ScanState, "idle">): Promise<Status>
	{
		return this.answer(call, () =>
		{
			if (this.scan?.state === from)
			{
				this.scan.state = to;
			}

			return this.snapshot();
		});
	}

	private named(scanId: number): FakeScan
	{
		if (this.scan?.id !== scanId)
		{
			throw new BackendError("staleScan", "the results belong to an earlier scan");
		}

		return this.scan;
	}

	private running(what: string): FakeScan
	{
		if (this.scan === null || !isRunning(this.scan))
		{
			throw new Error(`the fake backend cannot ${what}: no scan is running`);
		}

		return this.scan;
	}

	private snapshot(): Status
	{
		const scan = this.scan;
		if (scan === null)
		{
			return { state: "idle", scan: null };
		}

		const reclaimableBytes = scan.groups.reduce((sum, group) => sum + group.reclaimableBytes, 0);
		const skippedByReason: Partial<Record<SkipReason, number>> = {};
		for (const { reason } of scan.skipped)
		{
			skippedByReason[reason] = (skippedByReason[reason] ?? 0) + 1;
		}

		const view: ScanView = {
			id: scan.id,
			roots: [...scan.roots],
			...scan.progress,
			groups: scan.groups.length,
			reclaimableBytes,
			skipped: scan.skipped.length,
			skippedByReason,
			summary: scan.state === "completed" || scan.state === "cancelled"
				? {
						filesSeen: scan.progress.filesSeen,
						emptyFiles: 0,
						candidates: scan.progress.filesTotal,
						filesHashed: scan.progress.filesDone,
						bytesHashed: scan.progress.bytesDone,
						groups: scan.groups.length,
						reclaimableBytes,
						skipped: scan.skipped.length,
					}
				: null,
			error: scan.error,
		};

		return { state: scan.state, scan: view };
	}
}

function isRunning(scan: FakeScan): boolean
{
	return scan.state === "scanning" || scan.state === "paused" || scan.state === "stopping";
}

function page<T>(rows: readonly T[], query: PageQuery): T[]
{
	return rows.slice(query.offset, query.offset + Math.min(query.limit, MAX_PAGE));
}

export function sampleGroup(size: number, reclaimableBytes: number): Omit<GroupView, "id">
{
	const file = (id: number, name: string) => ({
		id,
		path: `/sample/${String(size)}/${name}`,
		name,
		folder: `/sample/${String(size)}`,
		modifiedMs: 1_731_400_000_000,
		otherPaths: [],
		linkedElsewhere: false,
	});

	return {
		size,
		hash: size.toString(16).padStart(64, "0"),
		independence: "verified",
		reclaimableBytes,
		files: [file(0, "one.bin"), file(1, "two.bin")],
	};
}
