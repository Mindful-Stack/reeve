// web/src/lib/attach.svelte.ts: images added to a Markdown field (spec: Screenshots on items, §3). The controller
// uploads each file, writes a placeholder into the text and swaps it for the image once stored; the write that saves
// the text sends `ids()`, and the server claims those for the item (kb/ADR-0022 R2). The client never deletes an
// upload: one removed from the text before saving stays unclaimed until the server's sweep.
import type { Attachment } from 'svelte/attachments';
import { api, unwrap, ApiError } from '#lib/api/client.ts';

/** An upload this field knows: what the server answered, kept so `ids()` can tell which are still shown. */
export type AttachmentRef = { id: string; url: string };
export type PendingState = 'uploading' | 'failed' | 'added';
export type Pending = { token: string; name: string; state: PendingState; error: string | null; url: string | null };

export const MAX_BYTES = 10_485_760;
export const TYPES = ['image/png', 'image/jpeg', 'image/webp'];
export const MESSAGES = {
	type: 'only PNG, JPEG and WebP images can be attached',
	size: 'the file is larger than 10 MiB'
} as const;
const EXT: Record<string, string> = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/webp': 'webp' };

const pad = (n: number) => String(n).padStart(2, '0');
/** A clipboard image has no real name ("image.png" or none): it is named for when it was added, to the second. */
export function nameFor(file: File, now = new Date()): string {
	if (file.name && file.name !== 'image.png') return file.name;
	const stamp = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
	return `screenshot-${stamp}.${EXT[file.type] ?? 'png'}`;
}

/** `name`, or `name-2`, `name-3`… when it is taken already: two pastes in one second stay apart. */
export function unique(name: string, taken: Set<string>): string {
	if (!taken.has(name)) return name;
	const dot = name.lastIndexOf('.');
	const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ''];
	for (let n = 2; ; n++) if (!taken.has(`${stem}-${n}${ext}`)) return `${stem}-${n}${ext}`;
}

/** Markdown alt text from a filename: no extension, and `\`, `[` and `]` escaped. */
export const altFor = (name: string): string => name.replace(/\.[^.]+$/, '').replace(/[\\[\]]/g, '\\$&');

/** The placeholders a reload in mid-upload leaves in a restored draft, and their trailing newline. */
const ORPHAN = /!\[Uploading (?:\\.|[^\]\\])*‹u\d+›\]\(\)\n?/g;
export const stripOrphans = (text: string): string => text.replace(ORPHAN, '');

/** Refuses what the server would refuse, before uploading; a file whose type the OS left empty is the server's to
 *  judge. Null when it may go. */
export function precheck(file: File): string | null {
	if (file.type && !TYPES.includes(file.type)) return MESSAGES.type;
	if (file.size > MAX_BYTES) return MESSAGES.size;
	return null;
}

let seq = 0;
/** Every upload this tab stored, whichever field it was added in: an image line cut from one editor and pasted into
 *  another is still claimed by the write that saves it. */
const stored = new Map<string, AttachmentRef>();

/** The ids among `refs` and every upload this tab stored whose image `text` shows. */
export function shownIds(text: string, refs: AttachmentRef[] = []): string[] {
	const all = new Map([...stored, ...refs.map((r) => [r.id, r] as const)]);
	return [...all.values()].filter((r) => text.includes(`(${r.url})`)).map((r) => r.id);
}

export class ImageUploads {
	pending = $state<Pending[]>([]);
	added = $state<AttachmentRef[]>([]);
	/** For the `aria-live` region. */
	status = $state('');
	/** A getter, not `$derived`: a controller may be created inside an effect that is later destroyed (the Report
	 *  dialog's), and a derived owned by a destroyed effect stops updating. */
	get busy(): boolean {
		return this.pending.some((p) => p.state === 'uploading');
	}
	/** The field each upload was added in, to swap its placeholder in place. */
	#fields = new Map<string, HTMLTextAreaElement>();
	#leaving = false;

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

	/** The uploads still shown in the text: what the write sends as `attachments`. Any upload of this tab counts, so an
	 *  image moved from another field is claimed too. */
	ids(): string[] {
		return shownIds(this.get(), this.added);
	}

	/** The `attachments` field of a write: absent when there are none, so a write without images is unchanged. */
	body(): { attachments?: string[] } {
		const ids = this.ids();
		return ids.length ? { attachments: ids } : {};
	}

	/** After a successful save, and when an editor opens or is cancelled: the rows empty, the placeholders of uploads
	 *  still running leave the text, and nothing is deleted. */
	reset(): void {
		let text = this.get();
		for (const p of this.pending) if (p.state === 'uploading') text = text.replace(this.placeholder(p) + '\n', '').replace(this.placeholder(p), '');
		if (text !== this.get()) this.set(text);
		this.#fields.clear();
		this.pending = [];
		this.added = [];
		this.status = '';
	}

	/** Drops a row and takes its image out of the text (the placeholder, or the image line once added); the upload
	 *  itself is never deleted, it is left for the sweep. */
	remove(p: Pending): void {
		this.pending = this.pending.filter((x) => x.token !== p.token);
		const text = this.get();
		const lines = [this.placeholder(p)];
		if (p.url) lines.push(`![${altFor(p.name)}](${p.url})`);
		for (const line of lines) {
			if (text.includes(line)) {
				this.set(text.replace(line + '\n', '').replace(line, ''));
				return;
			}
		}
	}

	/** Uploads `files` in order, each behind a placeholder at the caret of `el` (or at the end of the text). */
	add(files: File[], el?: HTMLTextAreaElement | null): void {
		const ready: Pending[] = [];
		const sources = new Map<string, File>();
		const taken = new Set(this.pending.map((p) => p.name));
		for (const file of files) {
			const name = unique(nameFor(file), taken);
			taken.add(name);
			const refused = precheck(file);
			const p: Pending = { token: `‹u${++seq}›`, name, state: refused ? 'failed' : 'uploading', error: refused, url: null };
			this.pending = [...this.pending, p];
			if (refused) this.status = `${name}: ${refused}`;
			else {
				ready.push(p);
				sources.set(p.token, file);
			}
		}
		if (ready.length === 0) return;
		this.insert(ready.map((p) => this.placeholder(p)).join('\n'), el);
		if (el) for (const p of ready) this.#fields.set(p.token, el);
		this.guardLeaving();
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
			const end = el.selectionEnd ?? start;
			const before = el.value.slice(0, start);
			const after = el.value.slice(end);
			const lead = before === '' || before.endsWith('\n') ? '' : '\n';
			const trail = after === '' || after.startsWith('\n') ? '' : '\n';
			el.setRangeText(lead + text + trail, start, end, 'end');
			el.dispatchEvent(new Event('input', { bubbles: true }));
			return;
		}
		const cur = this.get();
		this.set(cur === '' || cur.endsWith('\n') ? cur + text : `${cur}\n${text}`);
	}

	/** Swaps `line` for `next` in the text: through the field it was added in when that is still there (so the caret,
	 *  scroll and undo stay as the user left them), else through the model. */
	private swap(p: Pending, line: string, next: string): void {
		const el = this.#fields.get(p.token);
		this.#fields.delete(p.token);
		const i = el && el.isConnected && !el.disabled ? el.value.indexOf(line) : -1;
		if (el && i >= 0) {
			el.setRangeText(next, i, i + line.length, 'preserve');
			el.dispatchEvent(new Event('input', { bubbles: true }));
			return;
		}
		const text = this.get();
		if (text.includes(line)) this.set(text.replace(line, next));
	}

	/** The leave warning lasts while any upload runs, whether or not its field is still on the page. */
	private readonly onLeave = (e: BeforeUnloadEvent) => {
		if (this.busy) e.preventDefault();
	};
	private guardLeaving(): void {
		if (typeof window === 'undefined') return;
		if (this.busy && !this.#leaving) {
			window.addEventListener('beforeunload', this.onLeave);
			this.#leaving = true;
		} else if (!this.busy && this.#leaving) {
			window.removeEventListener('beforeunload', this.onLeave);
			this.#leaving = false;
		}
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
			stored.set(ref.id, ref);
			// Removed or reset while it uploaded: nothing more to show for it.
			if (!this.pending.some((x) => x.token === p.token)) return;
			this.added = [...this.added, ref];
			this.onAdded?.(this.refs());
			// The placeholder was edited away meanwhile: the image is not put back; the upload stays unclaimed.
			this.swap(p, this.placeholder(p), `![${altFor(p.name)}](${dto.url})`);
			this.update(p, { state: 'added', url: dto.url });
			this.guardLeaving();
			this.status = `${p.name} added`;
		} catch (e) {
			if (!this.pending.some((x) => x.token === p.token)) return;
			const message = e instanceof ApiError && e.code !== 'transport' ? e.message : 'the image could not be uploaded; add it again';
			this.update(p, { state: 'failed', error: message });
			const line = this.placeholder(p);
			this.swap(p, this.get().includes(line + '\n') ? line + '\n' : line, '');
			this.guardLeaving();
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
			// A rich paste (an office or web selection: text, HTML and an image of it) pastes the text. A copied file
			// brings its name as plain text but no HTML: that one is the image.
			const types = [...(data.types ?? [])];
			if (types.includes('text/plain') && types.includes('text/html')) return;
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
		el.addEventListener('paste', onPaste);
		el.addEventListener('dragover', onDragOver);
		el.addEventListener('dragleave', onDragLeave);
		el.addEventListener('drop', onDrop);
		if (guards++ === 0) {
			window.addEventListener('dragover', guard);
			window.addEventListener('drop', guard);
		}
		return () => {
			el.removeEventListener('paste', onPaste);
			el.removeEventListener('dragover', onDragOver);
			el.removeEventListener('dragleave', onDragLeave);
			el.removeEventListener('drop', onDrop);
			if (--guards === 0) {
				window.removeEventListener('dragover', guard);
				window.removeEventListener('drop', guard);
			}
		};
	};
}
