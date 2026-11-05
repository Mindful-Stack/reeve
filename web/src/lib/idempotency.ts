/** A draft's idempotency key and the text last sent with it (null until a send was attempted). */
export type DraftKey = { key: string; sent: string | null };

export function newDraftKey(): DraftKey {
	return { key: crypto.randomUUID(), sent: null };
}

/**
 * The key to send `text` with, called at submit time. The server returns the first stored item for a key and writes
 * nothing, so a key belongs to the text it was first sent with: retrying the same text keeps the key (a retry cannot
 * duplicate), and text edited after a send was attempted gets a new key (the edit is not silently dropped). Before any
 * send, edits keep one key. Reset with `newDraftKey()` after a successful send or when the form moves to another item.
 */
export function draftKey(state: DraftKey, text: string): string {
	if (state.sent !== null && state.sent !== text) state.key = crypto.randomUUID();
	state.sent = text;
	return state.key;
}

export type Sent<T> = { kind: 'sent'; value: T } | { kind: 'busy' } | { kind: 'failed'; error: unknown };

/** One create form's sends. `send` refuses to start while a send is in flight (a double submit makes one request), takes
 *  its key from `draftKey` (the same text keeps its key across retries; text changed after a send gets a new one), and
 *  replaces the key after a success, so identical content created again is a new item. A failure keeps the key for the
 *  retry. Make one per form instance; a form that moves to another item makes a new one. */
export class CreateCycle {
	#draft = newDraftKey();
	#inFlight = false;

	get inFlight(): boolean {
		return this.#inFlight;
	}

	async send<T>(text: string, request: (idempotencyKey: string) => Promise<T>): Promise<Sent<T>> {
		if (this.#inFlight) return { kind: 'busy' };
		this.#inFlight = true;
		try {
			const value = await request(draftKey(this.#draft, text));
			this.#draft = newDraftKey();
			return { kind: 'sent', value };
		} catch (error) {
			return { kind: 'failed', error };
		} finally {
			this.#inFlight = false;
		}
	}
}
