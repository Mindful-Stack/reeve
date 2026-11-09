// @vitest-environment jsdom
// With the browser condition in vite.config.ts, jsdom gives the browser build of Svelte its DOM; `$effect` runs.
import { flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { connect, everyVisible, freshNote, live, onLive, onStream, resetStreamForTests, RECENT_MAX, RECENT_MS } from './events.svelte.ts';

/** Just enough of EventSource: listeners by name, the three handlers and the ready state. */
class FakeEventSource {
	static CONNECTING = 0;
	static OPEN = 1;
	static CLOSED = 2;
	static last: FakeEventSource;
	readyState = FakeEventSource.CONNECTING;
	onopen: (() => void) | null = null;
	onerror: (() => void) | null = null;
	closed = false;
	private listeners = new Map<string, (e: MessageEvent) => void>();
	constructor(public url: string) {
		FakeEventSource.last = this;
	}
	addEventListener(name: string, fn: (e: MessageEvent) => void) {
		this.listeners.set(name, fn);
	}
	emit(name: string, data: string, lastEventId = '') {
		this.listeners.get(name)?.(new MessageEvent(name, { data, lastEventId }));
	}
	close() {
		this.closed = true;
		this.readyState = FakeEventSource.CLOSED;
	}
}

const entry = { id: 7, action: 'moved', entity_type: 'story', entity_key: 'ABC-1' };

beforeEach(() => {
	vi.stubGlobal('EventSource', FakeEventSource);
	vi.useFakeTimers();
	live.recent = [];
	resetStreamForTests();
});
afterEach(() => {
	delete (document as unknown as Record<string, unknown>).visibilityState;
	vi.unstubAllGlobals();
	vi.useRealTimers();
});

describe('connect', () => {
	it('tracks the cursor, each entry and each (re)connect', () => {
		const dispose = connect();
		const es = FakeEventSource.last;
		expect(es.url).toBe('/api/v1/events');
		const v0 = live.version;
		es.onopen?.();
		expect(live.connected).toBe(true);
		expect(live.version).toBe(v0 + 1);
		es.emit('cursor', '5');
		expect(live.lastId).toBe(5);
		es.emit('activity', JSON.stringify(entry), '7');
		expect(live.lastId).toBe(7);
		expect(live.latest).toEqual(entry);
		expect(live.version).toBe(v0 + 2);
		// the browser is retrying: not signed out
		const closed = vi.fn();
		dispose();
		const again = connect(closed);
		const es2 = FakeEventSource.last;
		es2.onopen?.();
		es2.readyState = FakeEventSource.CONNECTING;
		es2.onerror?.();
		expect(live.connected).toBe(false);
		expect(closed).not.toHaveBeenCalled();
		// the browser gave up (the reconnect was refused): the caller is told
		es2.readyState = FakeEventSource.CLOSED;
		es2.onerror?.();
		expect(closed).toHaveBeenCalledOnce();
		again();
		expect(es2.closed).toBe(true);
	});

	const act = (es: FakeEventSource, key: string, actor = 'erik@x', id = 1) =>
		es.emit('activity', JSON.stringify({ ...entry, id, entity_key: key, actor }), String(id));

	it('shows a ring for exactly the window without a further event', () => {
		const dispose = connect();
		const es = FakeEventSource.last;
		let note: string | null = null;
		const stop = $effect.root(() => {
			const n = $derived(freshNote(live.recent, 'ABC-1', 'me@x'));
			$effect(() => {
				note = n;
			});
		});
		act(es, 'ABC-1');
		flushSync();
		vi.advanceTimersByTime(RECENT_MS - 1);
		flushSync();
		expect(note).toBe('erik moved this \u00b7 just now');
		vi.advanceTimersByTime(1);
		flushSync();
		expect(note).toBeNull();
		stop();
		dispose();
	});

	it('expires staggered arrivals one by one', () => {
		const dispose = connect();
		const es = FakeEventSource.last;
		act(es, 'ABC-1');
		vi.advanceTimersByTime(9000);
		act(es, 'ABC-2', 'erik@x', 2);
		vi.advanceTimersByTime(1000);
		expect(live.recent.map((r) => r.entry.entity_key)).toEqual(['ABC-2']);
		vi.advanceTimersByTime(9000);
		expect(live.recent).toEqual([]);
		dispose();
	});

	it('keeps one entry per key and actor, and at most RECENT_MAX', () => {
		const dispose = connect();
		const es = FakeEventSource.last;
		act(es, 'ABC-1');
		act(es, 'ABC-1', 'erik@x', 2);
		expect(live.recent).toHaveLength(1);
		for (let i = 0; i < 250; i++) act(es, `K-${i}`, 'erik@x', i + 3);
		expect(live.recent).toHaveLength(RECENT_MAX);
		dispose();
	});

	it('keeps one entry per actor when different actors change the same key', () => {
		const dispose = connect();
		const es = FakeEventSource.last;
		act(es, 'ABC-1', 'erik@x', 1);
		act(es, 'ABC-1', 'sara@x', 2);
		expect(live.recent.map((r) => r.entry.actor)).toEqual(['erik@x', 'sara@x']);
		dispose();
	});

	it('keeps one timer through a burst and none once the burst has expired', () => {
		const dispose = connect();
		const es = FakeEventSource.last;
		for (let i = 0; i < 20; i++) {
			act(es, `K-${i}`, 'erik@x', i + 1);
			expect(vi.getTimerCount()).toBe(1);
			vi.advanceTimersByTime(100);
		}
		expect(live.recent).toHaveLength(20);
		vi.advanceTimersByTime(RECENT_MS);
		expect(live.recent).toEqual([]);
		expect(vi.getTimerCount()).toBe(0);
		dispose();
	});

	it('clears the window and its timer when the stream closes with entries present', () => {
		const dispose = connect();
		const es = FakeEventSource.last;
		act(es, 'ABC-1');
		act(es, 'ABC-2', 'sara@x', 2);
		expect(live.recent).toHaveLength(2);
		es.readyState = FakeEventSource.CLOSED;
		es.onerror?.();
		expect(live.recent).toEqual([]);
		expect(vi.getTimerCount()).toBe(0);
		dispose();
	});

	it('leaves no timer and no window after dispose', () => {
		const dispose = connect();
		act(FakeEventSource.last, 'ABC-1');
		dispose();
		expect(vi.getTimerCount()).toBe(0);
		expect(live.recent).toEqual([]);
	});
});

describe('onLive', () => {
	it('refetches once, 500 ms after the last change, and not for the version it mounted with', () => {
		const refetch = vi.fn();
		const stop = $effect.root(() => onLive(refetch));
		flushSync();
		vi.advanceTimersByTime(1000);
		expect(refetch).not.toHaveBeenCalled();
		live.version++;
		flushSync();
		vi.advanceTimersByTime(300);
		live.version++;
		flushSync();
		vi.advanceTimersByTime(499);
		expect(refetch).not.toHaveBeenCalled();
		vi.advanceTimersByTime(1);
		expect(refetch).toHaveBeenCalledOnce();
		// unmounting cancels a pending refetch
		live.version++;
		flushSync();
		stop();
		vi.advanceTimersByTime(10_000);
		expect(refetch).toHaveBeenCalledOnce();
	});

	describe('visibility and max-wait', () => {
		let state: 'visible' | 'hidden';
		const show = (next: 'visible' | 'hidden') => {
			state = next;
			document.dispatchEvent(new Event('visibilitychange'));
		};
		const bump = () => {
			live.version++;
			flushSync();
		};
		beforeEach(() => {
			state = 'visible';
			Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
		});

		it('refetches nothing while hidden, then once when shown', () => {
			const refetch = vi.fn();
			const stop = $effect.root(() => onLive(refetch));
			flushSync();
			show('hidden');
			bump();
			bump();
			bump();
			vi.advanceTimersByTime(10_000);
			expect(refetch).not.toHaveBeenCalled();
			show('visible');
			vi.advanceTimersByTime(0);
			expect(refetch).toHaveBeenCalledOnce();
			vi.advanceTimersByTime(10_000);
			expect(refetch).toHaveBeenCalledOnce();
			stop();
		});

		it('cancels a scheduled refetch when hidden and refetches when shown', () => {
			const refetch = vi.fn();
			const stop = $effect.root(() => onLive(refetch));
			flushSync();
			bump();
			vi.advanceTimersByTime(200);
			show('hidden');
			vi.advanceTimersByTime(10_000);
			expect(refetch).not.toHaveBeenCalled();
			show('visible');
			vi.advanceTimersByTime(0);
			expect(refetch).toHaveBeenCalledOnce();
			stop();
		});

		it('does not refetch on show when nothing is stale', () => {
			const refetch = vi.fn();
			const stop = $effect.root(() => onLive(refetch));
			flushSync();
			show('hidden');
			show('visible');
			vi.advanceTimersByTime(10_000);
			expect(refetch).not.toHaveBeenCalled();
			stop();
		});

		it('keeps the trailing debounce', () => {
			const refetch = vi.fn();
			const stop = $effect.root(() => onLive(refetch));
			flushSync();
			bump();
			vi.advanceTimersByTime(499);
			expect(refetch).not.toHaveBeenCalled();
			vi.advanceTimersByTime(1);
			expect(refetch).toHaveBeenCalledOnce();
			vi.advanceTimersByTime(10_000);
			expect(refetch).toHaveBeenCalledOnce();
			stop();
		});

		it('caps the wait under a steady stream', () => {
			const refetch = vi.fn();
			const stop = $effect.root(() => onLive(refetch));
			flushSync();
			bump();
			for (let t = 100; t < 4999; t += 100) {
				vi.advanceTimersByTime(100);
				bump();
			}
			// 4 900 ms elapsed; bring it to 4 999
			vi.advanceTimersByTime(99);
			expect(refetch).not.toHaveBeenCalled();
			vi.advanceTimersByTime(1);
			expect(refetch).toHaveBeenCalledTimes(1);
			for (let t = 5000; t < 6000; t += 100) {
				vi.advanceTimersByTime(100);
				bump();
			}
			vi.advanceTimersByTime(10_000);
			expect(refetch).toHaveBeenCalledTimes(2);
			stop();
		});

		it('removes its listener on unmount', () => {
			const refetch = vi.fn();
			const stop = $effect.root(() => onLive(refetch));
			flushSync();
			show('hidden');
			bump();
			stop();
			show('visible');
			vi.advanceTimersByTime(10_000);
			expect(refetch).not.toHaveBeenCalled();
		});
	});
});

describe('everyVisible', () => {
	let state: 'visible' | 'hidden';
	const show = (next: 'visible' | 'hidden') => {
		state = next;
		document.dispatchEvent(new Event('visibilitychange'));
	};
	beforeEach(() => {
		state = 'visible';
		Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
	});

	it('runs every period while the tab is visible', () => {
		const run = vi.fn();
		const stop = $effect.root(() => everyVisible(run, 1000));
		flushSync();
		vi.advanceTimersByTime(999);
		expect(run).not.toHaveBeenCalled();
		vi.advanceTimersByTime(1);
		expect(run).toHaveBeenCalledTimes(1);
		vi.advanceTimersByTime(2000);
		expect(run).toHaveBeenCalledTimes(3);
		stop();
	});

	it('keeps no timer while hidden; when shown it runs at once if a period went by, else when it is due', () => {
		const run = vi.fn();
		const stop = $effect.root(() => everyVisible(run, 1000));
		flushSync();
		vi.advanceTimersByTime(400);
		show('hidden');
		expect(vi.getTimerCount()).toBe(0);
		vi.advanceTimersByTime(5000);
		expect(run).not.toHaveBeenCalled();
		show('visible');
		expect(run).toHaveBeenCalledTimes(1);
		// Due again a full period after that run, and a short hide in between does not move it.
		vi.advanceTimersByTime(300);
		show('hidden');
		show('visible');
		expect(run).toHaveBeenCalledTimes(1);
		vi.advanceTimersByTime(699);
		expect(run).toHaveBeenCalledTimes(1);
		vi.advanceTimersByTime(1);
		expect(run).toHaveBeenCalledTimes(2);
		stop();
	});

	it('stops, listener and timer, on unmount', () => {
		const run = vi.fn();
		const stop = $effect.root(() => everyVisible(run, 1000));
		flushSync();
		stop();
		expect(vi.getTimerCount()).toBe(0);
		show('hidden');
		show('visible');
		vi.advanceTimersByTime(10_000);
		expect(run).not.toHaveBeenCalled();
	});
});

describe('onStream', () => {
	it('passes every entry in order and reports only a reopen, not the first open', () => {
		const seen: string[] = [];
		let reopened = 0;
		const stop = onStream({ entry: (e) => seen.push(e.action), reopened: () => reopened++ });
		const dispose = connect();
		const es = FakeEventSource.last;
		es.onopen?.();
		es.emit('activity', JSON.stringify({ ...entry, action: 'access_removed' }), '8');
		es.emit('activity', JSON.stringify({ ...entry, action: 'moved' }), '9');
		expect(seen).toEqual(['access_removed', 'moved']);
		expect(reopened).toBe(0);
		es.onerror?.();
		es.onopen?.(); // the browser reconnected
		expect(reopened).toBe(1);
		dispose();
		connect(); // a new EventSource after the browser gave up is a reopen too
		FakeEventSource.last.onopen?.();
		expect(reopened).toBe(2);
		stop();
		FakeEventSource.last.emit('activity', JSON.stringify(entry), '10');
		expect(seen).toHaveLength(2);
	});
});
