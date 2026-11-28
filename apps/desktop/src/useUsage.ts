import { useCallback, useEffect, useState } from "react";

import type { Backend } from "./backend/backend";
import type { Status, UsageFileView, UsageHistory, UsagePage, UsageQuery } from "./backend/types";
import { BackendError } from "./backend/types";
import { isRunning, PAGE_ROWS } from "./useScan";

/** Years shown one by one; everything older is counted together. */
export const YEARS_SHOWN = 10;

export interface YearSpan
{
	year: number;
	/** The span before `year`, not the year itself: everything older than the years shown. */
	before: boolean;
	files: number;
	bytes: number;
}

export interface Usage
{
	/** Null until the scan has walked its folders, and while it has no file to tell of. */
	spans: YearSpan[] | null;
	/** Files with no date at all. They are in no year. */
	undated: number;
	/** The place in `spans` of the year on show. */
	selected: number | null;
	select: (span: number) => void;
	/** Files of the year on show, the largest first. */
	page: UsagePage | null;
	pageNumber: number;
	turn: (page: number) => void;
	error: Error | null;
}

/** Which of a file's times its last use is. */
export type UsedBy = "changed" | "created" | "opened";

/**
 * The time a last use came from. When two times are the same moment, the one
 * Windows records most surely is told: a change, then a creation, then an
 * opening.
 */
export function usedBy(file: UsageFileView): UsedBy
{
	if (file.modifiedMs === file.lastUsedMs)
	{
		return "changed";
	}

	return file.createdMs === file.lastUsedMs ? "created" : "opened";
}

/** The first moment of each of the last `years` years, by the clock of this computer, oldest first. */
export function yearEdges(now: Date, years = YEARS_SHOWN): number[]
{
	const first = now.getFullYear() - years + 1;

	return Array.from({ length: years }, (_, place) => new Date(first + place, 0, 1).getTime());
}

interface Kept<T>
{
	key: string;
	value: T;
}

interface Choice
{
	scanId: number;
	span: number;
	page: number;
}

function asError(failure: unknown): Error
{
	return failure instanceof Error ? failure : new Error(String(failure));
}

/**
 * Files arrive while the scan walks its folders, so the history is asked for
 * once the walk is over, once per scan. The years are those of that moment:
 * a window left open over New Year counts the next scan in the new years.
 */
export function useUsage(backend: Backend, status: Status): Usage
{
	const [history, setHistory] = useState<Kept<UsageHistory> | null>(null);
	const [files, setFiles] = useState<Kept<UsagePage> | null>(null);
	const [choice, setChoice] = useState<Choice | null>(null);
	const [error, setError] = useState<Error | null>(null);

	const scan = status.scan;
	const scanId = scan?.id ?? null;
	const walked = scan !== null && (scan.phase === "hashing" || !isRunning(status.state));

	// As in the rows of a scan: a refusal for an earlier scan is a race the
	// next status settles.
	const report = useCallback((failure: unknown): void =>
	{
		if (!(failure instanceof BackendError && failure.code === "staleScan"))
		{
			setError(asError(failure));
		}
	}, []);

	useEffect(() =>
	{
		if (scanId === null || !walked)
		{
			return;
		}

		let outdated = false;
		backend.history(scanId, { edgesMs: yearEdges(new Date()) }).then((answer) =>
		{
			if (!outdated)
			{
				setHistory({ key: String(scanId), value: answer });
				setError(null);
			}
		}, report);

		return () =>
		{
			outdated = true;
		};
	}, [backend, report, scanId, walked]);

	const kept = history !== null && history.key === String(scanId) ? history.value : null;
	// A scan with no file to tell of has no usage to show.
	const counted = kept !== null && (kept.spans.some(span => span.files > 0) || kept.undated.files > 0) ? kept : null;
	const spans = counted === null ? null : yearSpans(counted);
	const chosen = choice !== null && choice.scanId === scanId ? choice : null;
	const oldest = spans?.findIndex(span => span.files > 0) ?? -1;
	const selected = chosen?.span ?? (oldest < 0 ? null : oldest);
	const pageNumber = chosen?.page ?? 0;
	// A time on an edge belongs to the later span, as in the session.
	const fromMs = selected === null || selected === 0 ? undefined : counted?.edgesMs[selected - 1];
	const beforeMs = selected === null ? undefined : counted?.edgesMs[selected];
	const filesKey = `${String(scanId)}:${String(selected)}:${String(pageNumber)}`;

	useEffect(() =>
	{
		if (scanId === null || selected === null)
		{
			return;
		}

		const query: UsageQuery = {
			...(fromMs === undefined ? {} : { fromMs }),
			...(beforeMs === undefined ? {} : { beforeMs }),
			offset: pageNumber * PAGE_ROWS,
			limit: PAGE_ROWS,
		};
		let outdated = false;
		backend.filesUsed(scanId, query).then((answer) =>
		{
			if (!outdated)
			{
				setFiles({ key: filesKey, value: answer });
				setError(null);
			}
		}, report);

		return () =>
		{
			outdated = true;
		};
	}, [backend, beforeMs, filesKey, fromMs, pageNumber, report, scanId, selected]);

	return {
		spans,
		undated: counted?.undated.files ?? 0,
		selected,
		select: (span) =>
		{
			if (scanId !== null)
			{
				setChoice({ scanId, span, page: 0 });
			}
		},
		// A page of another year or another scan is never shown under this one.
		page: files !== null && files.key === filesKey ? files.value : null,
		pageNumber,
		turn: (page) =>
		{
			if (scanId !== null && selected !== null)
			{
				setChoice({ scanId, span: selected, page });
			}
		},
		error,
	};
}

function yearSpans(history: UsageHistory): YearSpan[]
{
	const { edgesMs, spans } = history;
	const yearOf = (edge: number | undefined): number => new Date(edge ?? 0).getFullYear();

	return spans.map((span, place) => ({
		// The span before the first edge is the one before the first year shown.
		year: yearOf(edgesMs[Math.max(place - 1, 0)]),
		before: place === 0,
		files: span.files,
		bytes: span.bytes,
	}));
}
