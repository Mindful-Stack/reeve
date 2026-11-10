// web/src/lib/attach.svelte.ts: images added to a Markdown field (spec: Screenshots on items, §3). The controller
// uploads each file, writes a placeholder into the text and swaps it for the image once stored; the write that saves
// the text sends `ids()`, and the server claims those for the item (kb/ADR-0022 R2). The client never deletes an
// upload: one removed from the text before saving stays unclaimed until the server's sweep.
import type { Attachment } from 'svelte/attachments';
import { api, unwrap, ApiError } from '#lib/api/client.ts';

/** An upload this field knows: what the server answered, kept so `ids()` can tell which are still shown. */
export type AttachmentRef = { id: string; url: string };
export type PendingState = 'uploading' | 'failed' | 'added';
export type Pending = { token: string; name: string; state: PendingState; error: string | null };

export const MAX_BYTES = 10_485_760;
export const TYPES = ['image/png', 'image/jpeg', 'image/webp'];
export const MESSAGES = {
	type: 'only PNG, JPEG and WebP images can be attached',
	size: 'the file is larger than 10 MiB'
} as const;
const EXT: Record<string, string> = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/webp': 'webp' };

const pad = (n: number) => String(n).padStart(2, '0');
/** A clipboard image has no real name ("image.png" or none): it is named for when it was added. */
export function nameFor(file: File, now = new Date()): string {
	if (file.name && file.name !== 'image.png') return file.name;
	const stamp = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}`;
	return `screenshot-${stamp}.${EXT[file.type] ?? 'png'}`;
}

/** Markdown alt text from a filename: no extension, and `\`, `[` and `]` escaped. */
export const altFor = (name: string): string => name.replace(/\.[^.]+$/, '').replace(/[\\[\]]/g, '\\$&');

/** The placeholders a reload in mid-upload leaves in a restored draft, and their trailing newline. */
const ORPHAN = /!\[Uploading [^\]]*‹u\d+›\]\(\)\n?/g;
export const stripOrphans = (text: string): string => text.replace(ORPHAN, '');

/** Refuses what the server would refuse, before uploading; a file whose type the OS left empty is the server's to
 *  judge. Null when it may go. */
export function precheck(file: File): string | null {
	if (file.type && !TYPES.includes(file.type)) return MESSAGES.type;
	if (file.size > MAX_BYTES) return MESSAGES.size;
	return null;
}

let seq = 0;

export class ImageUploads {
	pending = $state<Pending[]>([]);
	added = $state<AttachmentRef[]>([]);
	/** For the `aria-live` region. */
	status = $state('');
	busy = $derived(this.pending.some((p) => p.state === 'uploading'));

	/** `get` and `set` read and write the field's model; `known` are uploads this draft made before (a restored
	 *  Report); `onAdded` hears every upload stored, so a draft can keep the refs. */
	constructor(
		private get: () => string,
		private set: (text: string) => void,
		known: AttachmentRef[] = [],
		private onAdded?: (refs: AttachmentRef[]) => void
	) {
		this.added = [...known];
	}

	refs(): AttachmentRef[] {
		return [...this.added];
	}

	/** The uploads still shown in the text: what the write sends as `attachments`. */
	ids(): string[] {
		const text = this.get();
		return this.added.filter((r) => text.includes(`(${r.url})`)).map((r) => r.id);
	}

	/** The `attachments` field of a write: absent when there are none, so a write without images is unchanged. */
	body(): { attachments?: string[] } {
		const ids = this.ids();
		return ids.length ? { attachments: ids } : {};
	}

	/** After a successful save: the row empties and nothing is deleted. */
	reset(): void {
		this.pending = [];
		this.added = [];
		this.status = '';
	}

	/** Drops a row, and its placeholder if it is still uploading; the upload itself is left for the sweep. */
	remove(p: Pending): void {
		this.pending = this.pending.filter((x) => x.token !== p.token);
		const text = this.get();
		const line = this.placeholder(p);
		if (text.includes(line)) this.set(text.replace(line + '\n', '').replace(line, ''));
	}

	/** Uploads `files` in order, each behind a placeholder at the caret of `el` (or at the end of the text). */
	add(files: File[], el?: HTMLTextAreaElement | null): void {
		const ready: Pending[] = [];
		const sources = new Map<string, File>();
		for (const file of files) {
			const name = nameFor(file);
			const refused = precheck(file);
			const p: Pending = { token: `‹u${++seq}›`, name, state: refused ? 'failed' : 'uploading', error: refused };
			this.pending = [...this.pending, p];
			if (refused) this.status = `${name}: ${refused}`;
			else {
				ready.push(p);
				sources.set(p.token, file);
			}
		}
		if (ready.length === 0) return;
		this.insert(ready.map((p) => this.placeholder(p)).join('\n'), el);
		for (const p of ready) void this.upload(p, sources.get(p.token)!);
	}

	private placeholder(p: Pending): string {
		return `![Uploading ${altFor(p.name)}… ${p.token}]()`;
	}

	/** Writes `text` at the caret through `setRangeText` and an `input` event, so a `value=`/`oninput` field's model
	 *  follows and the browser's undo keeps the user's typing; without a field, appends on a line of its own. */
	private insert(text: string, el?: HTMLTextAreaElement | null): void {
		if (el && el.isConnected && !el.disabled) {
			const start = el.selectionStart ?? el.value.length;
			const before = el.value.slice(0, start);
			const lead = before === '' || before.endsWith('\n') ? '' : '\n';
			el.setRangeText(lead + text, start, el.selectionEnd ?? start, 'end');
			el.dispatchEvent(new Event('input', { bubbles: true }));
			return;
		}
		const cur = this.get();
		this.set(cur === '' || cur.endsWith('\n') ? cur + text : `${cur}\n${text}`);
	}

	private update(p: Pending, patch: Partial<Pending>): void {
		this.pending = this.pending.map((x) => (x.token === p.token ? { ...x, ...patch } : x));
	}

	private async upload(p: Pending, file: File): Promise<void> {
		this.status = `Uploading ${p.name}…`;
		try {
			const bytes = new Uint8Array(await file.arrayBuffer());
			const dto = await unwrap(
				api.POST('/api/v1/attachments', {
					params: { query: { filename: p.name, idempotency_key: crypto.randomUUID() } },
					// openapi-typescript types a binary body as `string`; the bytes go as they are.
					body: bytes as never,
					bodySerializer: (b: unknown) => b as BodyInit,
					headers: { 'Content-Type': 'application/octet-stream' }
				})
			);
			const ref = { id: dto.id, url: dto.url };
			this.added = [...this.added, ref];
			this.onAdded?.(this.refs());
			const text = this.get();
			const line = this.placeholder(p);
			// The placeholder was edited away meanwhile: the image is not put back; the upload stays unclaimed.
			if (text.includes(line)) this.set(text.replace(line, `![${altFor(p.name)}](${dto.url})`));
			this.update(p, { state: 'added' });
			this.status = `${p.name} added`;
		} catch (e) {
			const message = e instanceof ApiError ? e.message : 'the image could not be uploaded';
			this.update(p, { state: 'failed', error: message });
			const text = this.get();
			const line = this.placeholder(p);
			if (text.includes(line)) this.set(text.replace(line + '\n', '').replace(line, ''));
			this.status = `${p.name}: ${message}`;
		}
	}
}

/** The images among a transfer's files. */
const imagesOf = (files: FileList | File[] | undefined | null): File[] => [...(files ?? [])];

// One window-level guard while any attachable field is mounted: a file dropped beside a field, or on a disabled one,
// must not navigate the tab away and lose what was typed.
let guards = 0;
const guard = (e: DragEvent) => {
	if (e.dataTransfer?.types && [...e.dataTransfer.types].includes('Files')) e.preventDefault();
};

/** Wires a textarea to `uploads`: an image paste (with no text on the clipboard), a file drop, the drop style and the
 *  leave warning while an upload runs. */
export function attachable(uploads: ImageUploads): Attachment<HTMLTextAreaElement> {
	return (el) => {
		const onPaste = (e: ClipboardEvent) => {
			const data = e.clipboardData;
			if (!data) return;
			// A rich paste (text with an image of it) pastes the text.
			if ([...(data.types ?? [])].includes('text/plain')) return;
			const files = imagesOf(data.files).filter((f) => f.type.startsWith('image/') || f.type === '');
			if (files.length === 0) return;
			e.preventDefault();
			uploads.add(files, el);
		};
		const onDragOver = (e: DragEvent) => {
			if (e.dataTransfer?.types && [...e.dataTransfer.types].includes('Files')) {
				e.preventDefault();
				el.dataset.drop = 'on';
			}
		};
		const onDragLeave = () => delete el.dataset.drop;
		const onDrop = (e: DragEvent) => {
			delete el.dataset.drop;
			const files = imagesOf(e.dataTransfer?.files);
			if (files.length === 0) return;
			e.preventDefault();
			uploads.add(files, el);
		};
		const onLeave = (e: BeforeUnloadEvent) => {
			if (uploads.busy) e.preventDefault();
		};
		el.addEventListener('paste', onPaste);
		el.addEventListener('dragover', onDragOver);
		el.addEventListener('dragleave', onDragLeave);
		el.addEventListener('drop', onDrop);
		window.addEventListener('beforeunload', onLeave);
		if (guards++ === 0) {
			window.addEventListener('dragover', guard);
			window.addEventListener('drop', guard);
		}
		return () => {
			el.removeEventListener('paste', onPaste);
			el.removeEventListener('dragover', onDragOver);
			el.removeEventListener('dragleave', onDragLeave);
			el.removeEventListener('drop', onDrop);
			window.removeEventListener('beforeunload', onLeave);
			if (--guards === 0) {
				window.removeEventListener('dragover', guard);
				window.removeEventListener('drop', guard);
			}
		};
	};
}
