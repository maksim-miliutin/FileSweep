export async function copyText(value: string): Promise<void>
{
	// Absent outside a secure context. WebView2 counts the window's
	// tauri.localhost origin as one; a plain http page elsewhere would not be.
	// eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
	if (navigator.clipboard === undefined)
	{
		throw new Error("the clipboard is not available here");
	}

	await navigator.clipboard.writeText(value);
}
