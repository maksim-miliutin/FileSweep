// The Rust side defines these shapes in filesweep-session; the examples in
// the contract folder hold both sides to them.
//
// A path is text for showing. Nothing is ever done to a file by its path: an
// action names a file by the ids of its scan, group and file.

export const SCAN_STATES = [
	"idle",
	"scanning",
	"paused",
	"stopping",
	"cancelled",
	"completed",
	"failed",
] as const;

export type ScanState = (typeof SCAN_STATES)[number];

export const PHASES = ["walking", "hashing"] as const;

export type Phase = (typeof PHASES)[number];

export const SKIP_REASONS = [
	"link",
	"onlineOnly",
	"system",
	"hidden",
	"special",
	"notADirectory",
	"alreadyCovered",
	"accessDenied",
	"inUse",
	"vanished",
	"changed",
	"io",
] as const;

export type SkipReason = (typeof SKIP_REASONS)[number];

export const INDEPENDENCE = ["verified", "unverified"] as const;

/** `unverified`: two entries of a group may be one file on disk, so no saving is promised. */
export type Independence = (typeof INDEPENDENCE)[number];

export const ROOT_KINDS = ["downloads", "desktop", "documents", "pictures"] as const;

export type RootKind = (typeof ROOT_KINDS)[number];

export interface SuggestedRoot
{
	kind: RootKind;
	path: string;
}

export interface Status
{
	state: ScanState;
	/** The running scan or the last one. Null only while idle. */
	scan: ScanView | null;
}

export interface ScanView
{
	/** Names this scan in every query about its results. */
	id: number;
	roots: string[];
	phase: Phase;
	filesSeen: number;
	filesDone: number;
	filesTotal: number;
	bytesDone: number;
	bytesTotal: number;
	groups: number;
	reclaimableBytes: number;
	skipped: number;
	skippedByReason: Partial<Record<SkipReason, number>>;
	/** Present once the scanner has returned. */
	summary: Summary | null;
	/** Present in the failed state. */
	error: string | null;
}

export interface Summary
{
	filesSeen: number;
	emptyFiles: number;
	candidates: number;
	filesHashed: number;
	bytesHashed: number;
	groups: number;
	reclaimableBytes: number;
	skipped: number;
}

export interface GroupPage
{
	/** All groups of the scan so far, not only the ones on this page. */
	total: number;
	offset: number;
	groups: GroupView[];
}

export interface GroupView
{
	/** The place of the group in the order the scanner found it. */
	id: number;
	/** Bytes of content in each file. */
	size: number;
	hash: string;
	independence: Independence;
	reclaimableBytes: number;
	files: FileView[];
}

export interface FileView
{
	/** The place of the file in its group. */
	id: number;
	path: string;
	name: string;
	folder: string;
	/** Milliseconds since 1970, negative before it. Null when the file system keeps no such time. */
	modifiedMs: number | null;
	/** More names of the same file inside the scan: hard links. */
	otherPaths: string[];
	/** The file has names outside the scanned folders too, so removing it here frees nothing. */
	linkedElsewhere: boolean;
}

export interface SkipPage
{
	total: number;
	offset: number;
	items: SkipView[];
}

export interface SkipView
{
	path: string;
	reason: SkipReason;
	/** What the operating system said. Only the `io` reason carries it. */
	detail: string | null;
}

export type GroupSort = "savings" | "found";

export interface GroupQuery
{
	offset: number;
	limit: number;
	sort: GroupSort;
}

export interface PageQuery
{
	offset: number;
	limit: number;
}

export const BACKEND_ERROR_CODES = [
	"busy",
	"noRoots",
	"tooManyRoots",
	"relativeRoot",
	"spawn",
	"staleScan",
	"noSuchFile",
	"revealFailed",
] as const;

/** `unexpected` is everything that is not a refusal the session knows how to make. */
export type BackendErrorCode = (typeof BACKEND_ERROR_CODES)[number] | "unexpected";

export class BackendError extends Error
{
	readonly code: BackendErrorCode;
	/** What came with the code: the limit, the offending path, the system's message. */
	readonly detail: Readonly<Record<string, unknown>>;

	constructor(code: BackendErrorCode, message: string, detail: Readonly<Record<string, unknown>> = {})
	{
		super(message);
		this.name = "BackendError";
		this.code = code;
		this.detail = detail;
	}
}
