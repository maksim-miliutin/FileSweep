// A TypeScript type promises nothing at run time, so everything that arrives
// from the backend is checked here: a message that is not what the type says
// stops with the name of the field that is wrong. Each decoder builds its
// answer field by field, so a field the interface does not know about never
// slips through.

import type {
	BackendErrorCode,
	FileView,
	GroupPage,
	GroupView,
	ScanView,
	SkipPage,
	SkipReason,
	SkipView,
	Status,
	SuggestedRoot,
	Summary,
	UsageFileView,
	UsageHistory,
	UsagePage,
	UsageSpan,
} from "./types";
import {
	BACKEND_ERROR_CODES,
	BackendError,
	INDEPENDENCE,
	PHASES,
	ROOT_KINDS,
	SCAN_STATES,
	SKIP_REASONS,
} from "./types";

export class DecodeError extends Error
{
	constructor(message: string)
	{
		super(message);
		this.name = "DecodeError";
	}
}

type Decoder<T> = (value: unknown, at: string) => T;

type Fields = Record<string, unknown>;

function refuse(at: string, expected: string, value: unknown): never
{
	const got = value === null ? "null" : Array.isArray(value) ? "an array" : typeof value;

	throw new DecodeError(`${at}: expected ${expected}, got ${got}`);
}

const text: Decoder<string> = (value, at) =>
	typeof value === "string" ? value : refuse(at, "text", value);

const flag: Decoder<boolean> = (value, at) =>
	typeof value === "boolean" ? value : refuse(at, "true or false", value);

/** A whole number JavaScript holds exactly. */
const integer: Decoder<number> = (value, at) =>
	typeof value === "number" && Number.isSafeInteger(value) ? value : refuse(at, "a whole number", value);

const count: Decoder<number> = (value, at) =>
{
	const whole = integer(value, at);

	return whole >= 0 ? whole : refuse(at, "a number that is not negative", "a negative number");
};

function orNull<T>(inner: Decoder<T>): Decoder<T | null>
{
	return (value, at) => (value === null ? null : inner(value, at));
}

function listOf<T>(inner: Decoder<T>): Decoder<T[]>
{
	return (value, at) =>
	{
		if (!Array.isArray(value))
		{
			return refuse(at, "a list", value);
		}

		return value.map((item, index) => inner(item, `${at}[${String(index)}]`));
	};
}

function oneOf<const Options extends readonly string[]>(options: Options): Decoder<Options[number]>
{
	return (value, at) =>
	{
		const known = options.find(option => option === value);

		return known ?? refuse(at, `one of ${options.join(", ")}`, value);
	};
}

function fields(value: unknown, at: string): Fields
{
	if (typeof value !== "object" || value === null || Array.isArray(value))
	{
		return refuse(at, "an object", value);
	}

	return value as Fields;
}

function read<T>(owner: Fields, at: string, name: string, decoder: Decoder<T>): T
{
	return decoder(owner[name], `${at}.${name}`);
}

const summary: Decoder<Summary> = (value, at) =>
{
	const from = fields(value, at);

	return {
		filesSeen: read(from, at, "filesSeen", count),
		emptyFiles: read(from, at, "emptyFiles", count),
		candidates: read(from, at, "candidates", count),
		filesHashed: read(from, at, "filesHashed", count),
		bytesHashed: read(from, at, "bytesHashed", count),
		groups: read(from, at, "groups", count),
		reclaimableBytes: read(from, at, "reclaimableBytes", count),
		skipped: read(from, at, "skipped", count),
	};
};

const reasonCounts: Decoder<Partial<Record<SkipReason, number>>> = (value, at) =>
{
	const reason = oneOf(SKIP_REASONS);
	const counts: Partial<Record<SkipReason, number>> = {};

	for (const [name, counted] of Object.entries(fields(value, at)))
	{
		counts[reason(name, `${at}.${name}`)] = count(counted, `${at}.${name}`);
	}

	return counts;
};

const scanView: Decoder<ScanView> = (value, at) =>
{
	const from = fields(value, at);

	return {
		id: read(from, at, "id", count),
		roots: read(from, at, "roots", listOf(text)),
		phase: read(from, at, "phase", oneOf(PHASES)),
		filesSeen: read(from, at, "filesSeen", count),
		filesDone: read(from, at, "filesDone", count),
		filesTotal: read(from, at, "filesTotal", count),
		bytesDone: read(from, at, "bytesDone", count),
		bytesTotal: read(from, at, "bytesTotal", count),
		groups: read(from, at, "groups", count),
		reclaimableBytes: read(from, at, "reclaimableBytes", count),
		skipped: read(from, at, "skipped", count),
		skippedByReason: read(from, at, "skippedByReason", reasonCounts),
		summary: read(from, at, "summary", orNull(summary)),
		error: read(from, at, "error", orNull(text)),
	};
};

const fileView: Decoder<FileView> = (value, at) =>
{
	const from = fields(value, at);

	return {
		id: read(from, at, "id", count),
		path: read(from, at, "path", text),
		name: read(from, at, "name", text),
		folder: read(from, at, "folder", text),
		modifiedMs: read(from, at, "modifiedMs", orNull(integer)),
		otherPaths: read(from, at, "otherPaths", listOf(text)),
		linkedElsewhere: read(from, at, "linkedElsewhere", flag),
	};
};

const groupView: Decoder<GroupView> = (value, at) =>
{
	const from = fields(value, at);

	return {
		id: read(from, at, "id", count),
		size: read(from, at, "size", count),
		hash: read(from, at, "hash", text),
		independence: read(from, at, "independence", oneOf(INDEPENDENCE)),
		reclaimableBytes: read(from, at, "reclaimableBytes", count),
		files: read(from, at, "files", listOf(fileView)),
	};
};

const skipView: Decoder<SkipView> = (value, at) =>
{
	const from = fields(value, at);

	return {
		path: read(from, at, "path", text),
		reason: read(from, at, "reason", oneOf(SKIP_REASONS)),
		detail: read(from, at, "detail", orNull(text)),
	};
};

export function decodeStatus(value: unknown): Status
{
	const from = fields(value, "status");

	return {
		state: read(from, "status", "state", oneOf(SCAN_STATES)),
		scan: read(from, "status", "scan", orNull(scanView)),
	};
}

export function decodeGroupPage(value: unknown): GroupPage
{
	const from = fields(value, "groups");

	return {
		total: read(from, "groups", "total", count),
		offset: read(from, "groups", "offset", count),
		groups: read(from, "groups", "groups", listOf(groupView)),
	};
}

export function decodeSkipPage(value: unknown): SkipPage
{
	const from = fields(value, "skipped");

	return {
		total: read(from, "skipped", "total", count),
		offset: read(from, "skipped", "offset", count),
		items: read(from, "skipped", "items", listOf(skipView)),
	};
}

const usageSpan: Decoder<UsageSpan> = (value, at) =>
{
	const from = fields(value, at);

	return {
		files: read(from, at, "files", count),
		bytes: read(from, at, "bytes", count),
	};
};

export function decodeUsageHistory(value: unknown): UsageHistory
{
	const from = fields(value, "history");

	return {
		edgesMs: read(from, "history", "edgesMs", listOf(integer)),
		spans: read(from, "history", "spans", listOf(usageSpan)),
		undated: read(from, "history", "undated", usageSpan),
	};
}

const usageFileView: Decoder<UsageFileView> = (value, at) =>
{
	const from = fields(value, at);

	return {
		id: read(from, at, "id", count),
		path: read(from, at, "path", text),
		name: read(from, at, "name", text),
		folder: read(from, at, "folder", text),
		size: read(from, at, "size", count),
		lastUsedMs: read(from, at, "lastUsedMs", integer),
		modifiedMs: read(from, at, "modifiedMs", orNull(integer)),
		accessedMs: read(from, at, "accessedMs", orNull(integer)),
		createdMs: read(from, at, "createdMs", orNull(integer)),
	};
};

export function decodeUsagePage(value: unknown): UsagePage
{
	const from = fields(value, "files");

	return {
		total: read(from, "files", "total", count),
		bytes: read(from, "files", "bytes", count),
		offset: read(from, "files", "offset", count),
		files: read(from, "files", "files", listOf(usageFileView)),
	};
}

const suggestedRoot: Decoder<SuggestedRoot> = (value, at) =>
{
	const from = fields(value, at);

	return {
		kind: read(from, at, "kind", oneOf(ROOT_KINDS)),
		path: read(from, at, "path", text),
	};
};

export function decodeSuggestedRoots(value: unknown): SuggestedRoot[]
{
	return listOf(suggestedRoot)(value, "suggestions");
}

/** For a command that answers with nothing when it succeeds. */
export function decodeNothing(value: unknown): void
{
	if (value !== null && value !== undefined)
	{
		refuse("answer", "nothing", value);
	}
}

const SAID: Record<Exclude<BackendErrorCode, "unexpected">, string> = {
	busy: "a scan is already running",
	noRoots: "no folder was given to scan",
	tooManyRoots: "too many folders were given to scan",
	relativeRoot: "a folder path is not absolute",
	spawn: "the scan could not be started",
	staleScan: "the results belong to an earlier scan",
	noSuchFile: "the scan has no such group or file",
	revealFailed: "the file could not be shown in its folder",
};

/**
 * A refusal of the session arrives as an object with a code. Anything else,
 * such as the plain text Tauri answers with when it cannot find a command or
 * read an argument, becomes `unexpected` and keeps what was said.
 */
export function decodeError(refusal: unknown): BackendError
{
	if (refusal instanceof BackendError)
	{
		return refusal;
	}

	if (typeof refusal === "object" && refusal !== null && "code" in refusal)
	{
		const { code, ...detail } = refusal as Fields;
		const known = BACKEND_ERROR_CODES.find(candidate => candidate === code);
		if (known !== undefined)
		{
			return new BackendError(known, SAID[known], detail);
		}
	}

	return new BackendError("unexpected", said(refusal));
}

function said(refusal: unknown): string
{
	if (refusal instanceof Error)
	{
		return refusal.message;
	}

	return typeof refusal === "string" ? refusal : JSON.stringify(refusal);
}
