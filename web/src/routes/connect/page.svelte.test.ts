// @vitest-environment jsdom
// The approval page: each state, and what Approve and Refuse send. The real API client runs against a fake fetch, so
// the 401 path is `unwrap`'s own.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { connectionRequest } from '#lib/test/fixtures.ts';
import { failure, settle } from '#lib/test/net.ts';

const route = vi.hoisted(() => ({ go: (_path: string) => {} }));
vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	const fake = fakePage({ url: '/connect?request=abc' });
	route.go = (path: string) => void (fake.page.url.href = new URL(path, 'http://localhost').href);
	return { page: fake.page };
});
type Sent = { method: string; path: string; body: unknown; client: string | null };
const net = vi.hoisted(() => ({
	calls: [] as { method: string; path: string; body: unknown; client: string | null }[],
	/** What a read answers: a value (an ok envelope), a Response, or a throw (a network failure). */
	read: (() => null) as () => unknown,
	/** What Approve or Refuse answers. */
	answer: (() => null) as () => unknown
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(({ method, path, body, headers }) => {
		net.calls.push({ method, path, body, client: headers.get('x-reeve-client') });
		return (method === 'GET' ? net.read : net.answer)();
	});
	return orig();
});
const { default: Page } = await import('./+page.svelte');

const assign = vi.fn();
let app: ReturnType<typeof mount> | null = null;
beforeEach(() => {
	vi.stubGlobal('location', { pathname: '/connect', search: '?request=abc', assign, href: '' });
	net.read = () => connectionRequest();
	net.answer = () => ({ redirect_to: 'https://callback.example/cb?code=c&state=s&iss=http%3A%2F%2Flocalhost' });
});
afterEach(() => {
	if (app) unmount(app);
	app = null;
	net.calls.length = 0;
	assign.mockReset();
	vi.unstubAllGlobals();
	route.go('/connect?request=abc');
	document.body.innerHTML = '';
});

const text = () => document.body.textContent ?? '';
const button = (name: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === name) as HTMLButtonElement | undefined;
const radio = (value: string) => document.querySelector(`input[type="radio"][value="${value}"]`) as HTMLInputElement;
const writes = (): Sent[] => net.calls.filter((c) => c.method !== 'GET');
async function open(path = '/connect?request=abc') {
	route.go(path);
	app = mount(Page, { target: document.body });
	await settle();
	flushSync();
}
async function shown() {
	await open();
	await vi.waitFor(() => expect(button('Approve')).toBeDefined());
}

describe('the approval page', () => {
	// kb/ADR-0029 R4: the signed-in email, the client, the host it redirects to. Catches a view built from anything but
	// the server's answer (the fixture's three hosts differ from each other and from the page's own).
	it('shows who is signed in, which app asks, and where the browser goes next', async () => {
		await shown();
		expect(net.calls).toEqual([{ method: 'GET', path: '/api/v1/connection-requests/abc', body: null, client: 'web' }]);
		expect(document.querySelector('h1')?.textContent).toBe('client.example wants to connect to localhost');
		expect(text()).toContain('Example App');
		expect(text()).toContain('Signed in as dan@example.com');
		expect(text()).toContain('You will be sent back to callback.example.');
		expect(text()).toContain('If someone sent you this link, refuse.');
		expect(text()).toContain('Approving gives their client.example account access to localhost as you.');
	});

	// Neither choice is preselected (a client asks for every scope the server lists). Catches a default scope.
	it('preselects nothing and keeps Approve disabled until a choice is made', async () => {
		await shown();
		expect(radio('read').checked).toBe(false);
		expect(radio('write').checked).toBe(false);
		expect(button('Approve')!.disabled).toBe(true);
		// A screen reader is told why: the disabled button is described by the choice it waits for.
		const describedBy = button('Approve')!.getAttribute('aria-describedby');
		expect(describedBy && document.getElementById(describedBy)?.textContent).toBe('What may it do?');
		expect(button('Refuse')!.disabled).toBe(false);
		radio('write').click();
		flushSync();
		expect(button('Approve')!.disabled).toBe(false);
	});

	// Catches an approve that sends another scope than the one chosen, drops the approval value, or navigates
	// somewhere other than the address the server answered.
	it('Approve posts the value and the chosen scope, then navigates to the answered address', async () => {
		await shown();
		radio('read').click();
		flushSync();
		button('Approve')!.click();
		await vi.waitFor(() => expect(assign).toHaveBeenCalled());
		expect(writes()).toEqual([{ method: 'POST', path: '/api/v1/connection-requests/abc/approve', body: { approval: 'value-1', scope: 'read' }, client: 'web' }]);
		expect(assign).toHaveBeenCalledWith('https://callback.example/cb?code=c&state=s&iss=http%3A%2F%2Flocalhost');
		expect(button('Approve')!.disabled).toBe(true); // still leaving: no second answer
	});

	it('Refuse needs no choice and posts the value alone', async () => {
		net.answer = () => ({ redirect_to: 'https://callback.example/cb?error=access_denied' });
		await shown();
		button('Refuse')!.click();
		await vi.waitFor(() => expect(assign).toHaveBeenCalledWith('https://callback.example/cb?error=access_denied'));
		expect(writes()).toEqual([{ method: 'POST', path: '/api/v1/connection-requests/abc/refuse', body: { approval: 'value-1' }, client: 'web' }]);
	});

	// Catches a page that navigates wherever the server says: a script address must never be assigned.
	it('does not navigate to an address that is neither https nor loopback http', async () => {
		net.answer = () => ({ redirect_to: 'javascript:alert(1)' });
		await shown();
		button('Refuse')!.click();
		await vi.waitFor(() => expect(text()).toContain('This connection request was not valid. Nothing was shared.'));
		expect(assign).not.toHaveBeenCalled();
	});

	// Catches a link to anything but the configured statement, one that opens in this tab or hands the statement's
	// site this page as its opener or referrer, and a provider sentence that depends on the link.
	it('links the privacy statement in a new tab when the instance has one', async () => {
		net.read = () => connectionRequest({ privacy_url: 'https://example.com/privacy' });
		await shown();
		const link = [...document.querySelectorAll('a')].find((a) => a.textContent?.trim() === 'Privacy statement');
		expect(link?.getAttribute('href')).toBe('https://example.com/privacy');
		expect(link?.getAttribute('target')).toBe('_blank');
		expect(link?.getAttribute('rel')).toBe('noopener noreferrer');
		expect(text()).toContain("What you read through this app is sent to the app's provider.");
	});

	it('shows no privacy link when the instance has none, and still says where what you read goes', async () => {
		for (const privacy_url of [undefined, null]) {
			net.read = () => connectionRequest({ privacy_url });
			await shown();
			expect(text()).not.toContain('Privacy statement');
			expect(document.querySelector('a')).toBeNull();
			expect(text()).toContain("What you read through this app is sent to the app's provider.");
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
		}
	});

	it('says the request expired when the read answers 404, and offers no buttons', async () => {
		net.read = () => failure('not found', 404, 'not_found');
		await open();
		await vi.waitFor(() => expect(text()).toContain('This connection request has expired or was opened in another browser. Go back to the app and connect again.'));
		expect(button('Approve')).toBeUndefined();
		expect(button('Refuse')).toBeUndefined();
	});

	it('says the request expired when the answer comes back 404', async () => {
		net.answer = () => failure('not found', 404, 'not_found');
		await shown();
		button('Refuse')!.click();
		await vi.waitFor(() => expect(text()).toContain('has expired or was opened in another browser'));
		expect(assign).not.toHaveBeenCalled();
	});

	// Catches buttons that stay disabled after a failure (the person could never retry) and a retry that sends nothing.
	it('keeps the buttons and says so when the server cannot be reached, and a second click tries again', async () => {
		net.answer = () => {
			throw new TypeError('fetch failed');
		};
		await shown();
		button('Refuse')!.click();
		await vi.waitFor(() => expect(text()).toContain('Could not reach localhost. Try again.'));
		expect(button('Refuse')!.disabled).toBe(false);
		net.answer = () => ({ redirect_to: 'https://callback.example/cb?error=access_denied' });
		button('Refuse')!.click();
		await vi.waitFor(() => expect(assign).toHaveBeenCalled());
		expect(writes()).toHaveLength(2);
	});

	// Catches a loading message a screen reader is not told about.
	it('announces that it is loading', async () => {
		net.read = () => new Promise(() => {});
		await open();
		expect(document.querySelector('[role="status"]')?.textContent).toBe('Loading…');
	});

	// Catches a failed read shown as "expired" (the person would give up on a request that is still good).
	it('says the server could not be reached when the read fails, and Try again reads again', async () => {
		net.read = () => failure('down', 500, 'internal');
		await open();
		await vi.waitFor(() => expect(text()).toContain('Could not reach localhost.'));
		expect(text()).not.toContain('expired');
		net.read = () => connectionRequest();
		button('Try again')!.click();
		await vi.waitFor(() => expect(button('Approve')).toBeDefined());
	});

	// A person without a session is sent to sign in with `next` set to this page's own address. `unwrap` does it; the
	// page must call the API through it.
	it('sends a signed-out person to sign in and back here', async () => {
		net.read = () => failure('unauthenticated', 401, 'unauthenticated');
		await open();
		await vi.waitFor(() => expect(assign).toHaveBeenCalledWith('/auth/signin?next=%2Fconnect%3Frequest%3Dabc'));
	});

	// The two problems authorize reports, and an address with neither parameter: a message, and no API call at all.
	it.each([
		['/connect?problem=app_not_enabled', 'This app is not enabled on localhost. Ask an admin.'],
		['/connect?problem=invalid_request', 'This connection request was not valid. Nothing was shared.'],
		['/connect?problem=something_else', 'This connection request was not valid. Nothing was shared.'],
		['/connect', 'This connection request was not valid. Nothing was shared.'],
		['/connect?problem=', 'This connection request was not valid. Nothing was shared.']
	])('%s shows its message without asking the server', async (path, message) => {
		await open(path);
		expect(text()).toContain(message);
		expect(net.calls).toHaveLength(0);
		expect(button('Approve')).toBeUndefined();
	});
});
