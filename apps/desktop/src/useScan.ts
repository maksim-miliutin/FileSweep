import { useCallback, useEffect, useRef, useState } from "react";

import type { Backend, StartOptions } from "./backend/backend";
import type { GroupPage, ScanState, SkipPage, Status } from "./backend/types";
import { BackendError } from "./backend/types";

export const POLL_MS = 200;
export const FIRST_ROWS = 50;

const IDLE: Status = { state: "idle", scan: null };

export interface ScanHandle
{
	status: Status;
	/** Cleared by the next call that goes right. */
	error: Error | null;
	// Functions, not methods: each can be handed to a button as it is.
	start: (roots: readonly string[], options: StartOptions) => void;
	pause: () => void;
	resume: () => void;
	cancel: () => void;
}

/** The scanner has not returned yet: the state can still change by itself. */
export function isRunning(state: ScanState): boolean
{
	return state === "scanning" || state === "paused" || state === "stopping";
}

function asError(failure: unknown): Error
{
	return failure instanceof Error ? failure : new Error(String(failure));
}

/**
 * The state lives in the backend: this only keeps the last answer, and while
 * a scan runs it asks again every `POLL_MS`.
 */
export function useScan(backend: Backend): ScanHandle
{
	const [status, setStatus] = useState<Status>(IDLE);
	const [error, setError] = useState<Error | null>(null);

	// Answers can arrive in another order than the questions were asked: a
	// status asked for before a pause may come back after the pause was
	// answered. Each call takes a number, and an answer is used only if no
	// later call has been answered already.
	const asked = useRef(0);
	const answered = useRef(0);

	const ask = useCallback(async (call: () => Promise<Status>): Promise<void> =>
	{
		asked.current += 1;
		const ticket = asked.current;

		let answer: Status | Error;
		try
		{
			answer = await call();
		}
		catch (failure)
		{
			answer = asError(failure);
		}

		if (ticket < answered.current)
		{
			return;
		}

		answered.current = ticket;
		if (answer instanceof Error)
		{
			setError(answer);
		}
		else
		{
			setStatus(answer);
			setError(null);
		}
	}, []);

	// A scan may already be running: the window can be reloaded under it.
	useEffect(() =>
	{
		void ask(() => backend.status());
	}, [ask, backend]);

	const running = isRunning(status.state);
	useEffect(() =>
	{
		if (!running)
		{
			return;
		}

		let stopped = false;
		let timer: ReturnType<typeof setTimeout> | undefined;

		// The next look is scheduled only when the last one has been
		// answered, so a slow backend is never asked twice at once.
		const lookLater = (): void =>
		{
			timer = setTimeout(() =>
			{
				void ask(() => backend.status()).then(() =>
				{
					if (!stopped)
					{
						lookLater();
					}
				});
			}, POLL_MS);
		};
		lookLater();

		return () =>
		{
			stopped = true;
			clearTimeout(timer);
		};
	}, [ask, backend, running]);

	return {
		status,
		error,
		start: (roots, options) => void ask(() => backend.start(roots, options)),
		pause: () => void ask(() => backend.pause()),
		resume: () => void ask(() => backend.resume()),
		cancel: () => void ask(() => backend.cancel()),
	};
}

export interface Rows
{
	/** The first groups of the scan, or null while it has none. */
	groups: GroupPage | null;
	skipped: SkipPage | null;
	error: Error | null;
}

interface Kept<Page>
{
	scanId: number;
	page: Page;
}

/**
 * While the scan runs, groups come in the order they are found, so rows do
 * not jump around; afterwards the most reclaimable come first.
 */
export function useRows(backend: Backend, status: Status): Rows
{
	const [groups, setGroups] = useState<Kept<GroupPage> | null>(null);
	const [skipped, setSkipped] = useState<Kept<SkipPage> | null>(null);
	const [error, setError] = useState<Error | null>(null);

	const scanId = status.scan?.id ?? null;
	const groupCount = status.scan?.groups ?? 0;
	const skipCount = status.scan?.skipped ?? 0;
	const sort = isRunning(status.state) ? "found" : "savings";

	// A page asked for an earlier scan can be refused as stale when a new
	// scan has started in between. That is a race the next status settles,
	// not something to show.
	const report = useCallback((failure: unknown): void =>
	{
		if (!(failure instanceof BackendError && failure.code === "staleScan"))
		{
			setError(asError(failure));
		}
	}, []);

	useEffect(() =>
	{
		if (scanId === null || groupCount === 0)
		{
			return;
		}

		let outdated = false;
		backend.groups(scanId, { offset: 0, limit: FIRST_ROWS, sort }).then((page) =>
		{
			if (!outdated)
			{
				setGroups({ scanId, page });
				setError(null);
			}
		}, report);

		return () =>
		{
			outdated = true;
		};
	}, [backend, groupCount, report, scanId, sort]);

	useEffect(() =>
	{
		if (scanId === null || skipCount === 0)
		{
			return;
		}

		let outdated = false;
		backend.skipped(scanId, { offset: 0, limit: FIRST_ROWS }).then((page) =>
		{
			if (!outdated)
			{
				setSkipped({ scanId, page });
				setError(null);
			}
		}, report);

		return () =>
		{
			outdated = true;
		};
	}, [backend, report, scanId, skipCount]);

	// Rows kept from an earlier scan are never shown for the current one.
	return {
		groups: groups !== null && groups.scanId === scanId && groupCount > 0 ? groups.page : null,
		skipped: skipped !== null && skipped.scanId === scanId && skipCount > 0 ? skipped.page : null,
		error,
	};
}
