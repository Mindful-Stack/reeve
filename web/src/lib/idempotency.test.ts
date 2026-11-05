import { describe, expect, it } from 'vitest';
import { CreateCycle, draftKey, newDraftKey } from './idempotency.ts';

describe('draftKey', () => {
	it('keeps one key across edits before any send', () => {
		const s = newDraftKey();
		const first = s.key;
		// Nothing was sent yet, so nothing calls draftKey; the key minted with the draft is the one used.
		expect(draftKey(s, 'pick A')).toBe(first);
	});

	it('keeps the key when the same text is sent again after a failed send', () => {
		const s = newDraftKey();
		const first = draftKey(s, 'pick A');
		expect(draftKey(s, 'pick A')).toBe(first);
		expect(draftKey(s, 'pick A')).toBe(first);
	});

	it('mints a new key when the text was edited after a failed send', () => {
		const s = newDraftKey();
		const first = draftKey(s, 'pick A');
		const second = draftKey(s, 'pick B');
		expect(second).not.toBe(first);
		// A retry of the edited text keeps the new key.
		expect(draftKey(s, 'pick B')).toBe(second);
	});

	it('gives a fresh state a different key', () => {
		expect(newDraftKey().key).not.toBe(newDraftKey().key);
	});
});

/** One item per key: a repeat returns the first item, as the planner's creates do. `lose` drops the next answer after
 *  the item is stored (a timeout or a dropped connection the server never noticed). */
function fakeServer() {
	const items = new Map<string, number>();
	const keys: string[] = [];
	let lose = false;
	return {
		items, keys,
		loseNext: () => (lose = true),
		create: async (key: string) => {
			keys.push(key);
			if (!items.has(key)) items.set(key, items.size + 1);
			if (lose) {
				lose = false;
				throw new TypeError('network error');
			}
			return items.get(key)!;
		}
	};
}

describe('CreateCycle', () => {
	it('creates identical content twice after a success: each create gets its own key', async () => {
		const server = fakeServer();
		const cycle = new CreateCycle();
		expect(await cycle.send('same text', server.create)).toEqual({ kind: 'sent', value: 1 });
		expect(await cycle.send('same text', server.create)).toEqual({ kind: 'sent', value: 2 });
		expect(new Set(server.keys).size).toBe(2);
		expect(server.items.size).toBe(2);
	});

	it('refuses a second submit while the first is in flight: one request, one item', async () => {
		const server = fakeServer();
		const cycle = new CreateCycle();
		let release!: () => void;
		const gate = new Promise<void>((r) => (release = r));
		const slow = async (key: string) => {
			await gate;
			return server.create(key);
		};
		const first = cycle.send('text', slow);
		expect(cycle.inFlight).toBe(true);
		expect(await cycle.send('text', slow)).toEqual({ kind: 'busy' });
		release();
		expect(await first).toEqual({ kind: 'sent', value: 1 });
		expect(cycle.inFlight).toBe(false);
		expect(server.keys).toHaveLength(1);
		expect(server.items.size).toBe(1);
	});

	it('retries a lost answer with the same key: one item', async () => {
		const server = fakeServer();
		const cycle = new CreateCycle();
		server.loseNext();
		const lost = await cycle.send('text', server.create);
		expect(lost.kind).toBe('failed');
		expect(await cycle.send('text', server.create)).toEqual({ kind: 'sent', value: 1 });
		expect(server.keys[0]).toBe(server.keys[1]);
		expect(server.items.size).toBe(1);
	});

	it('gives text edited after a failed send a new key', async () => {
		const server = fakeServer();
		const cycle = new CreateCycle();
		server.loseNext();
		await cycle.send('first', server.create);
		await cycle.send('edited', server.create);
		expect(server.keys[0]).not.toBe(server.keys[1]);
	});
});
