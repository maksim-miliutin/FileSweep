import type { Backend, StartOptions } from "./backend";
import type {
	GroupPage,
	GroupQuery,
	GroupView,
	HistoryQuery,
	PageQuery,
	Phase,
	ScanState,
	ScanView,
	SkipPage,
	SkipReason,
	SkipView,
	Status,
	SuggestedRoot,
	UsageFileView,
	UsageHistory,
	UsagePage,
	UsageQuery,
} from "./types";
import { BackendError } from "./types";

/** The most rows a page carries, as in the session. */
const MAX_PAGE = 500;

/** The most edges a history is counted between, as in the session. */
const MAX_EDGES = 64;

/** A file the walk indexed, with the times its listing gave. */
export interface IndexedSample
{
	path: string;
	size: number;
	modifiedMs: number | null;
	accessedMs: number | null;
	createdMs: number | null;
}

export interface RevealedIndexedFile
{
	scanId: number;
	fileId: number;
}

interface Progress
{
	phase: Phase;
	filesSeen: number;
	filesDone: number;
	filesTotal: number;
	bytesDone: number;
	bytesTotal: number;
}

export interface RevealedFile
{
	scanId: number;
	groupId: number;
	fileId: number;
}

interface FakeScan
{
	id: number;
	roots: string[];
	state: Exclude<ScanState, "idle">;
	progress: Progress;
	groups: GroupView[];
	skipped: SkipView[];
	/** In the order the walk met them: a file's place is its id. */
	files: IndexedSample[];
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
	readonly revealed: RevealedFile[] = [];
	readonly revealedIndexed: RevealedIndexedFile[] = [];
	picks: string[] = [];
	suggestions: SuggestedRoot[] = [];

	/** The options of the last start that was accepted. */
	startedWith: StartOptions | null = null;

	private scan: FakeScan | null = null;
	private lastId = 0;
	private failure: Error | null = null;

	/** The state as a status call would report it, without making a call. */
	get state(): ScanState
	{
		return this.scan?.state ?? "idle";
	}

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

	index(file: IndexedSample): void
	{
		this.running("index a file").files.push({ ...file });
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
				files: [],
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

	history(scanId: number, query: HistoryQuery): Promise<UsageHistory>
	{
		return this.answer("history", () =>
		{
			const edges = [...new Set(query.edgesMs)].sort((a, b) => a - b).slice(0, MAX_EDGES);
			const spans = Array.from({ length: edges.length + 1 }, () => ({ files: 0, bytes: 0 }));
			const undated = { files: 0, bytes: 0 };

			for (const file of this.named(scanId).files)
			{
				const used = lastUsed(file);
				// As many edges lie at or before the time as spans before the one it falls in.
				const span = used === null ? undated : spans[edges.filter(edge => edge <= used).length];
				if (span !== undefined)
				{
					span.files += 1;
					span.bytes += file.size;
				}
			}

			return { edgesMs: edges, spans, undated };
		});
	}

	filesUsed(scanId: number, query: UsageQuery): Promise<UsagePage>
	{
		return this.answer("filesUsed", () =>
		{
			const { fromMs, beforeMs } = query;
			const rows: UsageFileView[] = [];

			for (const [id, file] of this.named(scanId).files.entries())
			{
				const used = lastUsed(file);
				if (used !== null && (fromMs === undefined || used >= fromMs) && (beforeMs === undefined || used < beforeMs))
				{
					rows.push(usageView(id, file, used));
				}
			}

			// Array.sort is stable: files of one size stay in the order the walk met them.
			rows.sort((a, b) => b.size - a.size);

			return {
				total: rows.length,
				bytes: rows.reduce((sum, row) => sum + row.size, 0),
				offset: query.offset,
				files: page(rows, query),
			};
		});
	}

	pickFolders(): Promise<string[]>
	{
		return this.answer("pickFolders", () => [...this.picks]);
	}

	suggestedRoots(): Promise<SuggestedRoot[]>
	{
		return this.answer("suggestedRoots", () => this.suggestions.map(offer => ({ ...offer })));
	}

	reveal(scanId: number, groupId: number, fileId: number): Promise<void>
	{
		return this.answer("reveal", () =>
		{
			const group = this.named(scanId).groups.find(candidate => candidate.id === groupId);
			if (group?.files.some(file => file.id === fileId) !== true)
			{
				throw new BackendError("noSuchFile", "the scan has no such group or file");
			}

			this.revealed.push({ scanId, groupId, fileId });
		});
	}

	revealIndexed(scanId: number, fileId: number): Promise<void>
	{
		return this.answer("revealIndexed", () =>
		{
			if (this.named(scanId).files[fileId] === undefined)
			{
				throw new BackendError("noSuchFile", "the scan has no such group or file");
			}

			this.revealedIndexed.push({ scanId, fileId });
		});
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

/** The latest time the listing gave for the file, or null when it gave none. */
function lastUsed(file: IndexedSample): number | null
{
	const times = [file.modifiedMs, file.accessedMs, file.createdMs].filter(time => time !== null);

	return times.length === 0 ? null : Math.max(...times);
}

function usageView(id: number, file: IndexedSample, lastUsedMs: number): UsageFileView
{
	// The fake holds Windows paths and Unix ones alike: either separator ends the folder.
	const cut = Math.max(file.path.lastIndexOf("/"), file.path.lastIndexOf("\\"));

	return {
		id,
		path: file.path,
		name: file.path.slice(cut + 1),
		folder: cut < 0 ? "" : file.path.slice(0, cut),
		size: file.size,
		lastUsedMs,
		modifiedMs: file.modifiedMs,
		accessedMs: file.accessedMs,
		createdMs: file.createdMs,
	};
}

/** A file changed at `modifiedMs` and, as far as its listing says, not opened or made later. */
export function sampleFile(path: string, size: number, modifiedMs: number): IndexedSample
{
	return { path, size, modifiedMs, accessedMs: null, createdMs: null };
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

export function withOverrides(backend: Backend, overrides: Partial<Backend>): Backend
{
	return {
		start: (roots, options) => backend.start(roots, options),
		pause: () => backend.pause(),
		resume: () => backend.resume(),
		cancel: () => backend.cancel(),
		status: () => backend.status(),
		groups: (scanId, query) => backend.groups(scanId, query),
		skipped: (scanId, query) => backend.skipped(scanId, query),
		history: (scanId, query) => backend.history(scanId, query),
		filesUsed: (scanId, query) => backend.filesUsed(scanId, query),
		pickFolders: () => backend.pickFolders(),
		suggestedRoots: () => backend.suggestedRoots(),
		reveal: (scanId, groupId, fileId) => backend.reveal(scanId, groupId, fileId),
		revealIndexed: (scanId, fileId) => backend.revealIndexed(scanId, fileId),
		...overrides,
	};
}
