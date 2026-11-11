// Synthetic paste and drop events: jsdom has no ClipboardEvent, DataTransfer or DragEvent. Each is cancelable, so a
// test can tell a handled transfer (default prevented) from one left to the browser.

/** A transfer's shape as the handlers read it: its files and its types. */
function transfer(files: File[], types: string[]) {
	return { files, types: [...types, ...(files.length ? ['Files'] : [])], getData: () => '' };
}

export function pasteEvent(files: File[], { text = false, html = false } = {}): Event {
	const e = new Event('paste', { bubbles: true, cancelable: true });
	const types = [...(text ? ['text/plain'] : []), ...(html ? ['text/html'] : [])];
	Object.defineProperty(e, 'clipboardData', { value: transfer(files, types) });
	return e;
}

export function dragEvent(type: 'dragover' | 'drop', files: File[]): Event {
	const e = new Event(type, { bubbles: true, cancelable: true });
	Object.defineProperty(e, 'dataTransfer', { value: transfer(files, []) });
	return e;
}

/** A small file of `type`; `bytes` defaults to a PNG signature. */
export const file = (name: string, type = 'image/png', bytes: number[] = [0x89, 0x50, 0x4e, 0x47]) =>
	new File([new Uint8Array(bytes)], name, { type });
