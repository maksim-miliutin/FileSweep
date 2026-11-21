const UNITS = ["KB", "MB", "GB", "TB", "PB"] as const;

export function formatCount(count: number): string
{
	return count.toLocaleString("en-US");
}

/** A size the way Windows tells it: steps of 1024 under the names KB, MB and GB. */
export function formatBytes(bytes: number): string
{
	if (bytes < 1024)
	{
		return `${formatCount(bytes)} ${bytes === 1 ? "byte" : "bytes"}`;
	}

	let value = bytes / 1024;
	let unit = 0;
	// Compared as it will be shown, so that 1,023.97 KB becomes 1 MB and
	// not "1,024 KB".
	while (Math.round(value * 10) / 10 >= 1024 && unit < UNITS.length - 1)
	{
		value /= 1024;
		unit += 1;
	}

	return `${value.toLocaleString("en-US", { maximumFractionDigits: 1 })} ${UNITS[unit] ?? ""}`;
}
