// web/src/lib/promiseControl.svelte.ts: the promise items of an item's actions menu and the state behind them (the
// dialog, a done story's clear confirmation, the sprints the epic page fetches for the shortcuts), shared by the story
// drawer and the epic page (kb/ADR-0025). The host owns the write (its busy flag, notice and reload); this only says
// what is written where.
import { api, unwrap } from '#lib/api/client.ts';
import type { SetPromise, Sprints } from '#lib/api/types.ts';
import { promiseActions } from '#lib/promise.ts';
import type { MenuItem } from '#lib/ui/ActionsMenu.svelte';

export type PromiseHost = {
	/** Where the item's promise lives: `/api/v1/{path}/{key}/promise`. */
	path: 'stories' | 'epics';
	/** The item's key, once loaded. */
	key: () => string | undefined;
	/** The host's write: runs the action, reloads on success, leaves a failure's reason in the host's notice; true on
	 *  success. */
	write: (action: () => Promise<unknown>) => Promise<boolean>;
	/** The reason the last failed write left in the host's notice, taken out of it (the dialog shows it instead). */
	takeNotice: () => string | null;
	/** Focus back to the actions menu: the control that opened the write is gone. */
	refocus: () => Promise<void>;
	/** Fetch `GET /sprints` each time the dialog opens (the epic page); a host that holds the sprints already leaves it
	 *  off and passes its own to the dialog. */
	fetchSprints?: boolean;
};

export class PromiseControl {
	/** Whether the promise dialog is open, and the reason a failed save left in it. */
	open = $state(false);
	error = $state<string | null>(null);
	/** Whether a done story's clear is awaiting its confirmation, and the reason a failed clear left in it. */
	confirming = $state(false);
	confirmError = $state<string | null>(null);
	/** With `fetchSprints`: the sprints read when the dialog opened, null until they answer; `sprintsFailed` once the read
	 *  failed (the dialog then says shortcuts are unavailable instead of waiting). */
	sprints = $state<Sprints | null>(null);
	sprintsFailed = $state(false);

	#host: PromiseHost;
	/** Bumped whenever a sprints read in flight goes stale (closed, saved, another item). */
	#sprintsSeq = 0;

	constructor(host: PromiseHost) {
		this.#host = host;
	}

	/** The menu items for an item with (`hasOwn`) or without its own promise; a done story offers only Clear, which asks
	 *  first. An epic passes `done = false`. */
	items(hasOwn: boolean, done: boolean, busy: boolean): MenuItem[] {
		return promiseActions(hasOwn, done).map((a) => ({
			label: a.label,
			disabled: busy,
			onselect: a.kind === 'set' ? () => this.start() : done ? () => this.askClear() : () => void this.clear()
		}));
	}

	start() {
		this.error = null;
		this.open = true;
		if (!this.#host.fetchSprints) return;
		this.sprints = null;
		this.sprintsFailed = false;
		const seq = ++this.#sprintsSeq;
		unwrap(api.GET('/api/v1/sprints')).then(
			(sp) => {
				if (seq === this.#sprintsSeq && this.open) this.sprints = sp;
			},
			() => {
				if (seq === this.#sprintsSeq && this.open) this.sprintsFailed = true;
			}
		);
	}

	/** Closes the dialog, unless a save is in flight. */
	close(busy: boolean) {
		if (busy) return;
		this.open = false;
		this.#sprintsSeq++;
	}

	/** Everything closed, as when the host shows another item. */
	reset() {
		this.open = false;
		this.confirming = false;
		this.#sprintsSeq++;
	}

	async save(body: SetPromise) {
		const key = this.#host.key();
		if (!key) return;
		const params = { path: { key } };
		const ok = await this.#host.write(() =>
			this.#host.path === 'stories'
				? unwrap(api.POST('/api/v1/stories/{key}/promise', { params, body }))
				: unwrap(api.POST('/api/v1/epics/{key}/promise', { params, body }))
		);
		if (ok) {
			this.open = false;
			this.#sprintsSeq++;
			await this.#host.refocus();
		} else this.error = this.#host.takeNotice();
	}

	askClear() {
		this.confirmError = null;
		this.confirming = true;
	}

	cancelClear(busy: boolean) {
		if (!busy) this.confirming = false;
	}

	/** Clears the item's own promise; from the confirmation, a failure stays in it. */
	async clear() {
		const key = this.#host.key();
		if (!key) return;
		const params = { path: { key } };
		const ok = await this.#host.write(() =>
			this.#host.path === 'stories'
				? unwrap(api.DELETE('/api/v1/stories/{key}/promise', { params }))
				: unwrap(api.DELETE('/api/v1/epics/{key}/promise', { params }))
		);
		if (ok) {
			this.confirming = false;
			await this.#host.refocus();
		} else if (this.confirming) this.confirmError = this.#host.takeNotice();
	}
}
