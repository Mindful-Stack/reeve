// The end-to-end smoke (testing layer five): a real reeve-server with the static OIDC provider and a seeded fixture
// (scripts/web-smoke.sh). It is the only proof that the cookie, the gate, the drawer deep link and the override prompt
// work in a browser.
import { expect, test, type Locator, type Page } from '@playwright/test';

function env(name: string): string {
	const v = process.env[name];
	if (!v) throw new Error(`${name} is not set; run the smoke through scripts/web-smoke.sh`);
	return v;
}
const STORY = env('SMOKE_STORY');
const BLOCKED = env('SMOKE_BLOCKED_STORY');
const DECISION_STORY = env('SMOKE_DECISION_STORY');
// A discovery-complete epic: a chore under it is ready when its checks pass (an item without an epic is untriaged).
const EPIC = env('SMOKE_EPIC');

const headers = { 'X-Reeve-Client': 'web' };

/** Writes through the API as the signed-in browser (its cookie and the client header), for fixtures one test needs. */
async function post<T>(page: Page, path: string, data: unknown): Promise<T> {
	const res = await page.request.post(path, { headers, data });
	const text = await res.text();
	let body: { outcome: string; value: T };
	try {
		body = JSON.parse(text);
	} catch {
		throw new Error(`POST ${path}: ${res.status()} ${text}`);
	}
	expect(res.ok() && body.outcome, `POST ${path}: ${res.status()} ${text}`).toBe('ok');
	return body.value;
}
const chore = (page: Page, title: string, epic: string | null = EPIC) =>
	post<{ key: string }>(page, '/api/v1/stories', { idempotency_key: crypto.randomUUID(), title, kind: 'chore', epic, criteria: [], labels: [] });

// Sign in through the real browser flow against the static provider: /auth/login → provider → /auth/callback sets the
// cookie.
test.beforeEach(async ({ page, context }) => {
	await page.goto('/auth/login?next=/');
	await expect(page).toHaveURL(/\/$/);
	const session = (await context.cookies()).find((c) => c.name === 'reeve_session');
	expect(session).toBeDefined();
	expect(session!.httpOnly).toBe(true);
	expect(session!.sameSite).toBe('Lax');
	expect(session!.secure).toBe(false); // http on 127.0.0.1
});

test('planning view shows the seeded initiative in Now', async ({ page }) => {
	await page.goto('/');
	await expect(page.getByRole('heading', { name: 'Now' })).toBeVisible();
	await expect(page.getByText('Smoke initiative')).toBeVisible();
});

test('board shows ready and blocked stories and the drawer opens by url', async ({ page }) => {
	await page.goto('/board');
	await expect(page.getByTestId(`card-${STORY}`)).toContainText('ready');
	// Blocked (and untriaged) cards sit in the collapsed "Not ready" disclosure.
	await page.getByText(/^Not ready · \d+$/).click();
	await expect(page.getByTestId(`card-${BLOCKED}`)).toContainText('no criteria');
	await expect(page.getByTestId(`card-${DECISION_STORY}`)).toContainText('decision: no record');
	await page.goto('/board?story=' + STORY);
	await expect(page.getByRole('complementary')).toContainText(STORY);
});

test('board asks for recently done stories and labels the done column', async ({ page }) => {
	const req = page.waitForRequest((r) => new URL(r.url()).pathname === '/api/v1/stories' && r.method() === 'GET');
	await page.goto('/board');
	expect(new URL((await req).url()).searchParams.get('done')).toBe('recent');
	await expect(page.getByRole('heading', { name: /^done · last 14 days/ })).toBeVisible();
});

test('decisions view lists the open decision under All, the default', async ({ page }) => {
	await page.goto('/decisions');
	await expect(page.getByRole('group', { name: 'Decider' }).getByRole('button', { name: 'All' })).toHaveAttribute('aria-pressed', 'true');
	await expect(page.getByText('Smoke decision')).toBeVisible();
});

test('a move refused by the gate shows the override prompt and the override moves the story', async ({ page }) => {
	await page.goto('/board');
	// The blocked card sits in the collapsed "Not ready" disclosure; open it before each drag (the board reloads after the
	// cancel, so it may be closed again).
	const nr = page.locator('details.not-ready');
	const openNotReady = async () => {
		if ((await nr.getAttribute('open')) === null) await nr.locator('summary').click();
	};
	await openNotReady();
	const card = page.getByTestId(`card-${BLOCKED}`);
	await card.dragTo(page.getByTestId('column-in_progress'));
	const dialog = page.getByRole('dialog');
	// The prompt gives the full sentence for each reason ("no acceptance criteria"); the card's short form is "no criteria".
	await expect(dialog).toContainText('no acceptance criteria');
	// cancelling leaves the card in todo
	await dialog.getByRole('button', { name: 'Cancel' }).click();
	await expect(dialog).toBeHidden();
	await expect(page.getByTestId('column-todo')).toContainText(BLOCKED);
	await openNotReady();
	await card.dragTo(page.getByTestId('column-in_progress'));
	await page.getByRole('dialog').getByLabel('Reason').fill('smoke override');
	await page.getByRole('dialog').getByRole('button', { name: 'Move anyway' }).click();
	await expect(page.getByTestId('column-in_progress')).toContainText(BLOCKED);
	const log = await page.request.get('/api/v1/activity?action=override', { headers: { 'X-Reeve-Client': 'web' } });
	expect(log.ok()).toBe(true);
	const body = (await log.json()) as { value: { payload: { reason?: string } }[] };
	expect(body.value.some((e) => e.payload.reason === 'smoke override')).toBe(true);
});

test('a move made elsewhere reaches an open board over the event stream, without a reload', async ({ page, browser }) => {
	// The board loads once on mount and refetches once 500 ms after the stream opens. Wait for both before the other
	// browser moves the story, so the card can only reach in progress through an `activity` frame: with the stream's
	// `activity` listener removed (or the event renamed on the server) this test fails, since no later refetch happens.
	let loads = 0;
	page.on('response', (r) => {
		if (new URL(r.url()).pathname === '/api/v1/stories') loads++;
	});
	await page.goto('/board');
	await expect(page.getByText('live', { exact: true })).toBeAttached(); // visually hidden
	await expect.poll(() => loads).toBe(2);
	await expect(page.getByTestId('column-todo')).toContainText(STORY);
	// Another browser (its own session) moves the story through the API; this tab never navigates.
	const other = await browser.newContext({ baseURL: process.env.REEVE_URL });
	const p = await other.newPage();
	await p.goto('/auth/login?next=/');
	const headers = { 'X-Reeve-Client': 'web' };
	const story = (await (await p.request.get(`/api/v1/stories/${STORY}`, { headers })).json()) as {
		value: { version: number };
	};
	const moved = await p.request.post(`/api/v1/stories/${STORY}/move`, {
		headers,
		data: { to: 'in_progress', version: story.value.version }
	});
	expect(((await moved.json()) as { outcome: string }).outcome).toBe('ok');
	await other.close();
	// The stream polls every 2 s and the page refetches 500 ms after the entry: well inside the 5 s default timeout.
	await expect(page.getByTestId('column-in_progress')).toContainText(STORY);
	expect(loads).toBeGreaterThan(2);
});

test('signed out users are sent to sign in and see the button', async ({ browser }) => {
	const ctx = await browser.newContext();
	const p = await ctx.newPage();
	await p.goto('/board');
	await expect(p).toHaveURL(/\/auth\/signin\?next=%2Fboard/);
	await expect(p.getByRole('link', { name: 'Sign in with Google' })).toBeVisible();
	await ctx.close();
});

test('an open tab whose session dies is sent to sign in by the event stream', async ({ page }) => {
	// The board loads once on mount and refetches once 500 ms after the stream opens. Wait for both, so the redirect below
	// cannot come from that refetch answering 401: it has to come from the stream closing.
	let loads = 0;
	page.on('response', (r) => {
		if (new URL(r.url()).pathname === '/api/v1/stories') loads++;
	});
	await page.goto('/board');
	await expect(page.getByText('live', { exact: true })).toBeAttached(); // visually hidden
	await expect.poll(() => loads).toBe(2);
	// Revoke this tab's session (the newest browser session: beforeEach just signed in) and nothing else, so no other
	// revocation's activity entry reaches the stream first and triggers a refetch that would 401 on its own.
	const headers = { 'X-Reeve-Client': 'web' };
	const tokens = (await (await page.request.get('/api/v1/tokens', { headers })).json()) as {
		value: { id: number; label: string; revoked_at?: string | null }[];
	};
	const mine = Math.max(...tokens.value.filter((t) => t.label === 'web session' && !t.revoked_at).map((t) => t.id));
	expect((await page.request.delete(`/api/v1/tokens/${mine}`, { headers })).ok()).toBe(true);
	// The stream ends at the next poll tick, the browser's reconnect is refused, `/me` answers 401 and the layout sends
	// the tab to sign in: a 2 s tick plus the browser's reconnect delay.
	await expect(page).toHaveURL(/\/auth\/signin\?next=%2Fboard/, { timeout: 15_000 });
});

test('the drawer removes a block', async ({ page }) => {
	const blocked = await chore(page, 'Smoke blocked chore');
	const blocker = await chore(page, 'Smoke blocking chore');
	await post(page, `/api/v1/stories/${blocked.key}/blocks`, { blocker: blocker.key });
	await page.goto('/board?story=' + blocked.key);
	const drawer = page.getByRole('complementary');
	await drawer.getByRole('button', { name: `Remove block by ${blocker.key}` }).click();
	await expect(drawer).toContainText('Nothing blocks this story.');
	const after = (await (await page.request.get(`/api/v1/stories/${blocked.key}`, { headers })).json()) as {
		value: { blocked_by: string[] };
	};
	expect(after.value.blocked_by).toEqual([]);
});

test('a decision is edited in place: its decider changes', async ({ page }) => {
	const s = await chore(page, 'Smoke chore with a decision');
	const d = await post<{ id: number; version: number }>(page, '/api/v1/decisions', {
		idempotency_key: crypto.randomUUID(), on: s.key, title: 'Smoke decision to edit', decider: 'smoke@example.com'
	});
	// All is the default: the decision stays listed once its decider changes.
	await page.goto('/decisions');
	const row = page.getByRole('listitem').filter({ hasText: 'Smoke decision to edit' });
	await row.getByRole('button', { name: /^Actions for Smoke decision to edit on / }).click();
	await page.getByRole('menuitem', { name: 'Edit' }).click();
	const form = row.getByRole('form', { name: 'Edit decision Smoke decision to edit' });
	await form.getByLabel('Decider').fill('someone@example.com');
	await form.getByRole('button', { name: 'Save' }).click();
	await expect(form).toBeHidden();
	await expect(row).toContainText('someone');
	const after = (await (await page.request.get(`/api/v1/decisions/${d.id}`, { headers })).json()) as {
		value: { decider: string; version: number };
	};
	expect(after.value).toMatchObject({ decider: 'someone@example.com', version: d.version + 1 });
});

test('the decision queue: classify, blocks, withdraw and decide', async ({ page }) => {
	const epic = await post<{ key: string }>(page, '/api/v1/epics', { idempotency_key: crypto.randomUUID(), title: 'Smoke queue epic' });
	await post(page, '/api/v1/decisions', {
		idempotency_key: crypto.randomUUID(), on: epic.key, title: 'Smoke one-way decision', decider: 'smoke@example.com'
	});
	await page.goto('/decisions');
	const row = page.getByRole('listitem').filter({ hasText: 'Smoke one-way decision' });
	await row.getByRole('button', { name: 'Classify Smoke one-way decision' }).click();
	await page.getByRole('menuitem', { name: 'One-way' }).click();
	await expect(row.locator('.chip')).toHaveText('One-way');

	// A story under the decision's epic is held back by it: the queue says so after a reload.
	await post(page, '/api/v1/stories', {
		idempotency_key: crypto.randomUUID(), title: 'Smoke story behind the queue', kind: 'chore', criteria: ['it is decided'], labels: [], epic: epic.key
	});
	await page.reload();
	await expect(row.getByRole('button', { name: 'Blocks 1 story: Smoke one-way decision' })).toBeVisible();

	await row.getByRole('button', { name: /^Actions for Smoke one-way decision on / }).click();
	await expect(page.getByRole('menuitem', { name: 'Decide' })).toHaveCount(0); // a one-way decision is decided by its record
	await page.getByRole('menuitem', { name: 'Withdraw' }).click();
	const withdraw = row.getByRole('form', { name: 'Withdraw decision Smoke one-way decision' });
	await withdraw.getByLabel("Why it's no longer needed").fill('the epic was dropped');
	await withdraw.getByRole('button', { name: 'Withdraw' }).click();
	// The default view: the closed card stays in sight (focus is on it) though the Closed group is folded.
	await expect(row).toContainText('Withdrawn by');
	// On a fresh load nothing pins it: it is in the folded Closed group, which unfolds on request.
	await page.reload();
	const fold = page.getByRole('button', { name: /Closed/, expanded: false });
	await expect(fold).toBeVisible();
	await expect(row).toHaveCount(0);
	await fold.click();
	await expect(page.locator('#decisions-closed')).toContainText('Smoke one-way decision');

	await post(page, '/api/v1/decisions', {
		idempotency_key: crypto.randomUUID(), on: epic.key, title: 'Smoke two-way decision', decider: 'smoke@example.com', reversibility: 'two_way'
	});
	await page.reload();
	const two = page.getByRole('listitem').filter({ hasText: 'Smoke two-way decision' });
	await expect(two.locator('.chip')).toHaveText('Two-way');
	await two.getByRole('button', { name: /^Actions for Smoke two-way decision on / }).click();
	await page.getByRole('menuitem', { name: 'Decide' }).click();
	const decide = two.getByRole('form', { name: 'Decide decision Smoke two-way decision' });
	await decide.getByLabel("What was decided, and where it's written down", { exact: true }).fill('we go with the simple one');
	await decide.getByRole('button', { name: 'Decide' }).click();
	await expect(two).toContainText('Decided by');
});

test('the planning loop runs in the browser: initiative, epic, story, backlog', async ({ page }) => {
	await page.goto('/');
	await page.getByRole('button', { name: 'New initiative' }).click();
	const form = page.getByRole('dialog').getByRole('form', { name: 'New initiative' });
	await form.getByLabel('Title').fill('Smoke created initiative');
	await expect(form.getByLabel('Horizon')).toHaveValue('Now'); // the first configured bucket (reeve.smoke.toml)
	await form.getByLabel('Target month').fill('2026-13');
	await form.getByRole('button', { name: 'Create initiative' }).click();
	await expect(form.getByRole('alert')).toContainText('YYYY-MM');
	await expect(form.getByLabel('Title')).toHaveValue('Smoke created initiative'); // the draft is kept
	await form.getByLabel('Target month').fill('2026-12');
	await form.getByRole('button', { name: 'Create initiative' }).click();
	await expect(page).toHaveURL(/\/initiatives\/SMK-\d+$/);
	const iniKey = new URL(page.url()).pathname.split('/').pop()!;
	await expect(page.getByRole('heading', { level: 1, name: 'Smoke created initiative' })).toBeVisible();
	await expect(page.getByText('Dec 2026')).toBeVisible();
	const newEpic = page.getByRole('button', { name: 'New epic' });
	const epicForm = page.getByRole('dialog').getByRole('form', { name: 'New epic' });
	await newEpic.click();
	await epicForm.getByLabel('Title').fill('Smoke created epic');
	await epicForm.getByRole('button', { name: 'Create epic' }).click();
	const epicLink = page.getByRole('link', { name: 'Smoke created epic' });
	await expect(epicLink).toBeVisible();
	await expect(epicForm).toBeHidden(); // the dialog closes after success
	// The same content created twice is two epics: the key was replaced after the first success (CreateCycle).
	for (const n of [1, 2]) {
		await newEpic.click();
		await expect(epicForm.getByLabel('Title')).toHaveValue(''); // reset after success
		await epicForm.getByLabel('Title').fill('Smoke twin epic');
		await epicForm.getByRole('button', { name: 'Create epic' }).click();
		await expect(page.getByRole('link', { name: 'Smoke twin epic' })).toHaveCount(n);
	}
	await page.getByRole('button', { name: 'Edit details' }).click();
	const details = page.getByRole('form', { name: 'Initiative details' });
	await details.getByLabel('Owner').fill('smoke@example.com');
	await details.getByRole('button', { name: 'Save details' }).click();
	await expect(details).toBeHidden();
	// The meta line shows the owner (the header's account avatar carries the same title, so look inside main).
	const owner = page.getByRole('main').getByTitle('smoke@example.com');
	await expect(owner).toHaveText('smoke');
	await expect(owner).toBeVisible();
	await epicLink.click();
	await expect(page).toHaveURL(/\/epics\/SMK-\d+$/);
	await page.getByRole('button', { name: 'New story' }).click();
	const storyForm = page.getByRole('dialog').getByRole('form', { name: 'New story' });
	await storyForm.getByLabel('Title').fill('Smoke created story');
	await storyForm.getByRole('button', { name: 'Create story' }).click();
	await expect(page.getByRole('link', { name: 'Smoke created story' })).toBeVisible();
	// Move the epic to no initiative, then back through the banner.
	await page.getByRole('button', { name: 'Edit details' }).click();
	await page.getByRole('form', { name: 'Epic details' }).getByLabel('Initiative').selectOption('');
	await page.getByRole('form', { name: 'Epic details' }).getByRole('button', { name: 'Save details' }).click();
	await expect(page.getByText('This epic belongs to no initiative.')).toBeVisible();
	await page.getByRole('button', { name: 'Choose an initiative' }).click();
	await page.getByRole('form', { name: 'Epic details' }).getByLabel('Initiative').selectOption(iniKey);
	await page.getByRole('form', { name: 'Epic details' }).getByRole('button', { name: 'Save details' }).click();
	await expect(page.getByText('This epic belongs to no initiative.')).toBeHidden();
	await expect(page.getByRole('navigation', { name: 'Breadcrumb' }).getByRole('link', { name: iniKey })).toBeVisible();
	await page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: 'Backlog', exact: true }).click();
	await expect(page).toHaveURL(/\/backlog$/);
	await expect(page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: 'Backlog', exact: true })).toHaveAttribute('aria-current', 'page');
	const section = page.getByRole('region', { name: 'Smoke created initiative' });
	await expect(section.getByRole('link', { name: 'Smoke created epic' })).toBeVisible();
	await expect(section).toContainText('0 of 1 ready'); // its one story has no criteria
	await page.getByRole('button', { name: 'New epic' }).click();
	const indexForm = page.getByRole('dialog').getByRole('form', { name: 'New epic' });
	await indexForm.getByLabel('Title').fill('Smoke loose epic');
	await expect(indexForm.getByLabel('Initiative')).toHaveValue(''); // none by default
	await indexForm.getByRole('button', { name: 'Create epic' }).click();
	await expect(page.getByRole('region', { name: 'No initiative' }).getByRole('link', { name: 'Smoke loose epic' })).toBeVisible();
	// The old index redirects, also on a full load through the server's page fallback.
	await page.goto('/epics');
	await expect(page).toHaveURL(/\/backlog$/);
});

test('the backlog lists epic-less work under No epic and moves a row up through its menu', async ({ page }) => {
	const a = await chore(page, 'Smoke backlog first', null);
	const b = await chore(page, 'Smoke backlog second', null);
	await page.goto('/backlog?group=rank');
	const order = async () => await page.locator('ol.rows li').evaluateAll((li) => li.map((x) => x.getAttribute('data-key')));
	// The list loads after the page mounts: wait for both rows before reading the order.
	await expect(page.locator(`ol.rows li[data-key="${b.key}"]`)).toBeVisible();
	await expect(page.locator(`ol.rows li[data-key="${a.key}"]`)).toBeVisible();
	const before = await order();
	expect(before[before.indexOf(b.key) - 1]).toBe(a.key); // b sits right below a
	// The server's order, not only the DOM's.
	const ranks = async () => {
		const res = await page.request.get('/api/v1/stories?delivery=todo', { headers });
		expect(res.ok()).toBe(true);
		const body = (await res.json()) as { value: { story: { key: string; rank: number } }[] };
		const rank = (key: string) => body.value.find((s) => s.story.key === key)!.story.rank;
		return { a: rank(a.key), b: rank(b.key) };
	};
	const actions = page.getByRole('button', { name: `Actions for ${b.key}`, exact: true });
	await actions.click();
	await page.getByRole('menuitem', { name: 'Move up' }).click();
	await expect.poll(async () => (await order()).indexOf(b.key)).toBe(before.indexOf(b.key) - 1);
	const up = await ranks();
	expect(up.b).toBeLessThan(up.a);
	// After that write the ranks are distinct: Move down covers the distinct-rank path.
	await actions.click();
	await page.getByRole('menuitem', { name: 'Move down' }).click();
	await expect.poll(async () => (await order()).indexOf(b.key)).toBe(before.indexOf(b.key));
	const down = await ranks();
	expect(down.a).toBeLessThan(down.b);
	await page.goto('/backlog');
	await expect(page.getByRole('region', { name: 'No epic' })).toContainText('Smoke backlog first');
});

test('the client loads nothing from outside the server', async ({ page }) => {
	// Its own epic-less chore, so the No epic heading below exists when this test runs alone.
	const own = await chore(page, 'Smoke foreign chore', null);
	const origin = new URL(env('REEVE_URL')).origin;
	const foreign: string[] = [];
	page.on('request', (r) => {
		if (new URL(r.url()).origin !== origin) foreign.push(r.url());
	});
	// Wait for each page to render its data (not networkidle: the event stream never idles). Headings alone are not
	// enough: the loading placeholders carry them too, so each page waits for a seeded row or card.
	await page.goto('/');
	await expect(page.getByRole('heading', { level: 1, name: 'Planning' })).toBeVisible();
	await expect(page.getByRole('heading', { name: 'Now' })).toBeVisible();
	await expect(page.getByText('Smoke initiative')).toBeVisible();
	await page.goto('/backlog');
	await expect(page.getByRole('heading', { level: 1, name: /^Backlog/ })).toBeVisible();
	await expect(page.getByRole('heading', { level: 2, name: /^No epic/ })).toBeVisible();
	const noEpic = page.getByRole('region', { name: 'No epic' });
	await expect(noEpic.locator(`li[data-key="${own.key}"]`)).toContainText('Smoke foreign chore');
	await expect(noEpic.locator(`li[data-key="${own.key}"]`)).toBeVisible();
	await page.goto('/board');
	await expect(page.getByRole('heading', { level: 1, name: 'Board' })).toBeVisible();
	await expect(page.getByRole('heading', { name: /^done · last 14 days/ })).toBeVisible();
	await expect(page.getByTestId(`card-${STORY}`)).toBeVisible();
	// The web fonts are the last thing a page fetches; a foreign font request would land only after they settle.
	await page.evaluate(() => document.fonts.ready);
	expect(foreign).toEqual([]);
});

/** What holds focus, for the trace a failing focus test prints: the tag, its text and its href. */
const focused = (page: Page) =>
	page.evaluate(() => {
		const a = document.activeElement;
		if (!a || a === document.body) return 'body';
		return `${a.tagName.toLowerCase()} "${(a.textContent ?? '').trim().slice(0, 40)}" ${a.getAttribute('href') ?? ''}`.trim();
	});

for (const view of ['board', 'backlog'] as const) {
	test(`a story opened from the ${view} by keyboard gives focus back to its link, also after a blocker link`, async ({ page }) => {
		const opener = await chore(page, `Smoke ${view} focus opener`);
		const blocker = await chore(page, `Smoke ${view} focus blocker`);
		await post(page, `/api/v1/stories/${opener.key}/blocks`, { blocker: blocker.key });
		await page.goto('/' + view);
		let link: Locator;
		if (view === 'board') {
			// Blocked (and untriaged) cards sit in the collapsed "Not ready" disclosure.
			await page.getByText(/^Not ready · \d+$/).click();
			link = page.getByTestId(`card-${opener.key}`).getByRole('link', { name: `Smoke ${view} focus opener` });
		} else {
			link = page.locator(`li[data-key="${opener.key}"]`).getByRole('link', { name: `Smoke ${view} focus opener` });
		}
		const trace: string[] = [];
		const step = async (what: string) => trace.push(`${what}: ${await focused(page)}`);
		try {
			await link.focus();
			await expect(link).toBeFocused();
			await step('before Enter');
			await page.keyboard.press('Enter');
			const drawer = page.locator(`[aria-label="Story ${opener.key}"]`);
			await expect(drawer).toBeVisible();
			await expect(page).toHaveURL(new RegExp(`[?&]story=${opener.key}`));
			// The drawer's heading takes focus once the story loads; Escape is the panel's own key.
			await expect(drawer.locator('h2')).toBeFocused();
			await step('drawer open');
			await page.keyboard.press('Escape');
			await expect(drawer).toBeHidden();
			await expect(link).toBeFocused();
			await step('after Escape');

			// Again, then on to the blocker from inside the drawer: closing that drawer returns focus to the first opener.
			await page.keyboard.press('Enter');
			await expect(drawer.locator('h2')).toBeFocused();
			await step('drawer reopened');
			const blockerLink = drawer.getByRole('link', { name: blocker.key });
			await blockerLink.focus();
			await page.keyboard.press('Enter');
			const second = page.locator(`[aria-label="Story ${blocker.key}"]`);
			await expect(second.locator('h2')).toBeFocused();
			await step('blocker drawer open');
			await page.keyboard.press('Escape');
			await expect(second).toBeHidden();
			await expect(link).toBeFocused();
			await step('after second Escape');
		} catch (e) {
			await step('at failure');
			console.log(`focus trace (${view}):\n  ${trace.join('\n  ')}`);
			throw e;
		}
	});
}

test('the sprint cycle: a goal, the open sprint, the start nudge, confidence, close and history', async ({ page }) => {
	// A ready chore (criteria, under a discovery-complete epic) that serves no goal yet: moving it into progress asks for its goal.
	const work = await post<{ key: string }>(page, '/api/v1/stories', {
		idempotency_key: crypto.randomUUID(), title: 'Smoke sprint chore', kind: 'chore', epic: EPIC, criteria: ['it ships'], labels: []
	});
	// Under 40 characters, so the nudge and the card show it whole.
	const outcome = 'The smoke runs a sprint end to end';

	// The first goal creates the draft; no sprint is active, so the draft opens from its own panel.
	await page.goto('/sprint');
	const next = page.getByTestId('next-sprint');
	const add = next.getByRole('form', { name: 'New goal' });
	await add.getByLabel('Outcome').fill(outcome);
	await add.getByLabel("We'll know when").fill('the smoke closes it');
	await add.getByRole('button', { name: 'Add goal' }).click();
	await expect(next.getByRole('heading', { name: outcome })).toBeVisible();
	await next.getByRole('button', { name: 'Open sprint' }).click();
	const active = page.getByTestId('active-sprint');
	await expect(active.getByRole('heading', { name: outcome })).toBeVisible();

	// On the board, the card's own move control starts the chore; the nudge links it to the goal.
	await page.goto('/board');
	const card = page.getByTestId(`card-${work.key}`);
	await card.getByRole('button', { name: `Actions for ${work.key}`, exact: true }).click();
	await page.getByRole('menuitem', { name: 'Move to in progress' }).click();
	await expect(page.getByTestId('column-in_progress')).toContainText(work.key);
	const nudge = page.getByTestId('nudge');
	await expect(nudge).toContainText('Serving which goal?');
	await nudge.getByRole('button', { name: outcome }).click();
	await expect(nudge).toBeHidden();
	await expect(card).toContainText(`goal: ${outcome}`);
	const story = (await (await page.request.get(`/api/v1/stories/${work.key}`, { headers })).json()) as {
		value: { goals: { outcome: string }[] };
	};
	expect(story.value.goals.map((g) => g.outcome)).toEqual([outcome]);

	// Check Tuesday: record "at risk".
	await page.goto('/sprint');
	await expect(active.getByText('1 in flight')).toBeVisible();
	await active.getByRole('button', { name: 'Record confidence' }).click();
	const confidence = active.getByRole('form', { name: 'Record confidence' });
	await confidence.getByRole('radio', { name: 'at risk' }).check();
	await confidence.getByRole('button', { name: 'Record' }).click();
	await expect(active.getByTestId('confidence')).toHaveText('at risk');

	// Plan Tuesday: close with "met".
	await active.getByRole('button', { name: 'Close…' }).click();
	const dialog = page.getByRole('dialog');
	await dialog.getByRole('radio', { name: 'met', exact: true }).check();
	await dialog.getByRole('button', { name: 'Close sprint' }).click();
	await expect(dialog.getByRole('status')).toHaveText('The sprint is closed.');
	await dialog.getByRole('button', { name: 'Close', exact: true }).click();
	await expect(dialog).toBeHidden();
	await expect(active).toBeHidden();

	// History: the verdict, and the numbers with the membership rule. Nothing moved since the close, so the derived
	// counts stand alone: no "now · at close" pair and no "changed since close" note (blocked-by is always "at close").
	const history = page.locator('[data-testid^="history-"]').filter({ hasText: outcome });
	await expect(history).toContainText('confidence at risk');
	await expect(history).toContainText('verdict met');
	await history.getByRole('button', { name: 'Show numbers' }).click();
	const numbers = history.getByTestId('numbers');
	await expect(numbers).toContainText('Counts the work completed in this sprint; work started but not finished shows as in flight.');
	await expect(numbers).toContainText('1 in flight');
	await expect(numbers).toContainText('blocked by 0 decisions at close');
	await expect(numbers).not.toContainText('now ');
	await expect(numbers).not.toContainText('changed since close');
});

// A decision row is one line of aligned cells (DecisionList): at every desktop width each cell must hold its content,
// since a squeezed nowrap cell draws over its neighbour, and only a browser lays the columns out.
test('decision rows keep every cell within its column at desktop widths', async ({ page }) => {
	const s = await chore(page, 'Smoke chore for the row layout');
	await post(page, '/api/v1/decisions', {
		idempotency_key: crypto.randomUUID(),
		on: s.key,
		title: 'Smoke decision with a long title to lay out across the columns',
		decider: 'someone.with.a.long.name@example.com'
	});
	for (const width of [800, 980, 1200, 1280, 1440]) {
		await page.setViewportSize({ width, height: 900 });
		await page.goto('/decisions');
		await expect(page.getByText('Smoke decision with a long title')).toBeVisible();
		const spills = await page
			.locator('.decisions > li .row > *')
			.evaluateAll((cells) =>
				// A cell cut on purpose (the decider's ellipsis) is not spilling.
				cells.filter((c) => c.scrollWidth > c.clientWidth + 1 && getComputedStyle(c).textOverflow !== 'ellipsis').map((c) => `${c.getAttribute('data-col')}: ${c.textContent?.replace(/\s+/g, ' ').trim()}`)
			);
		expect(spills, `cells spilling at ${width}px`).toEqual([]);
		expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), `no page scroll at ${width}px`).toBe(true);
	}
});

test('the header goal picker links an initiative and, after navigating by click, one of its epics', async ({ page }) => {
	const outcome = `Picker smoke ${crypto.randomUUID().slice(0, 8)}`;
	await page.goto('/');
	await page.getByRole('button', { name: 'Goals ▾' }).click();
	const dialog = page.getByRole('dialog', { name: 'Sprint goals' });
	await dialog.getByRole('button', { name: '+ Add goal' }).click();
	const form = dialog.getByRole('form', { name: 'New goal' });
	await form.getByLabel('Outcome').fill(outcome);
	await form.getByLabel("We'll know when").fill('the smoke links it');
	await form.getByRole('button', { name: 'Add goal', exact: true }).click();

	await dialog.getByRole('button', { name: outcome }).click();
	await expect(dialog).toBeHidden();
	await expect(page.getByRole('button', { name: `Linking to: ${outcome}` })).toBeVisible();

	// The initiative, on Planning.
	const card = page.locator('[data-testid^="card-"]', { hasText: 'Smoke initiative' });
	const ini = (await card.getAttribute('data-testid'))!.replace('card-', '');
	const linkIni = card.getByRole('button', { name: `Link ${ini} to goal: ${outcome}`, exact: true });
	await linkIni.click();
	await expect(linkIni).toHaveAttribute('aria-pressed', 'true');

	// Client-side navigation keeps the mode: follow the card's title, then link an epic there.
	await card.getByRole('link', { name: 'Smoke initiative' }).click();
	await expect(page).toHaveURL(new RegExp(`/initiatives/${ini}$`));
	await expect(page.getByRole('button', { name: `Linking to: ${outcome}` })).toBeVisible();
	const epicCard = page.locator('li.epic-card', { hasText: 'Smoke epic A' });
	const linkEpic = epicCard.getByRole('button', { name: new RegExp(`^Link \\S+ to goal: ${outcome}$`) });
	const epic = (await linkEpic.getAttribute('aria-label'))!.split(' ')[1];
	await linkEpic.click();
	await expect(linkEpic).toHaveAttribute('aria-pressed', 'true');

	// The links are the server's and under this goal: /sprint loads fresh and lists both keys on this goal.
	await page.goto('/sprint');
	const goalCard = page.getByTestId('next-sprint').locator('article.goal', { has: page.getByRole('heading', { name: outcome, exact: true }) });
	await expect(goalCard.getByRole('button', { name: `Unlink ${ini}`, exact: true })).toBeVisible();
	await expect(goalCard.getByRole('button', { name: `Unlink ${epic}`, exact: true })).toBeVisible();
	// The full load ended linking mode.
	await expect(page.getByRole('button', { name: 'Goals ▾' })).toBeVisible();
});

test('an initiative marked urgent from its card shows in the War room with its reason, and End war room clears it', async ({ page }) => {
	const title = `Urgent smoke ${crypto.randomUUID().slice(0, 8)}`;
	const ini = await post<{ key: string }>(page, '/api/v1/initiatives', { idempotency_key: crypto.randomUUID(), title, horizon: 'Now' });
	await page.goto('/');
	const warRoom = page.locator('section[aria-label="War room"]');
	await expect(warRoom).toHaveCount(0); // nothing is marked yet

	await page.getByRole('button', { name: `Actions for ${ini.key}`, exact: true }).click();
	await page.getByRole('menuitem', { name: 'Mark urgent…' }).click();
	const dialog = page.getByRole('dialog', { name: `Mark ${ini.key} urgent` });
	await dialog.getByLabel('Reason (optional)').fill('the customer is waiting');
	await dialog.getByRole('button', { name: 'Mark urgent', exact: true }).click();
	await expect(dialog).toBeHidden();

	await expect(warRoom).toBeVisible();
	await expect(warRoom).toContainText('the customer is waiting');
	await expect(warRoom).toContainText(title);
	await expect(warRoom.getByRole('button', { name: `Clear urgent mark on ${ini.key} ${title}` })).toBeVisible();

	await warRoom.getByRole('button', { name: 'End war room' }).click();
	await expect(warRoom).toContainText('Clear 1 mark');
	await warRoom.locator('[data-confirm]').click();
	await expect(warRoom).toHaveCount(0);
});

const MEMBER_TOKEN = env('SMOKE_MEMBER_TOKEN');

test('an admin allows a guest email on the Access page and removes it', async ({ page }) => {
	const guest = `guest-${crypto.randomUUID().slice(0, 8)}@partner.example`;
	await page.goto('/access');
	await page.getByLabel('Guest email').fill(guest);
	await page.getByRole('form', { name: 'Allow a guest' }).getByRole('button', { name: 'Allow' }).click();
	const row = page.getByRole('listitem').filter({ hasText: guest });
	await expect(row).toContainText('added by smoke@example.com');
	await row.getByRole('button', { name: 'Remove' }).click();
	await expect(page.getByText(`Remove ${guest}? They lose access on their next request.`)).toBeVisible();
	await page.getByRole('button', { name: 'Yes, remove' }).click();
	await expect(row).toHaveCount(0);
});

test("a member's open tab gains and loses the Access link as an admin grants and revokes, without a reload", async ({ page, browser }) => {
	const member = await browser.newContext({ baseURL: process.env.REEVE_URL });
	await member.addCookies([{ name: 'reeve_session', value: MEMBER_TOKEN, url: process.env.REEVE_URL! }]);
	const b = await member.newPage();
	let navigations = 0;
	b.on('framenavigated', (f) => {
		if (f === b.mainFrame()) navigations++;
	});
	await b.goto('/tokens');
	await expect(b.getByText('live', { exact: true })).toBeAttached();
	const nav = b.getByRole('navigation', { name: 'Account' });
	await expect(nav.getByRole('link', { name: 'Access' })).toHaveCount(0);
	const loaded = navigations;
	const grant = await page.request.post('/api/v1/access/admins', { headers, data: { email: 'smoke-member@example.com' } });
	expect(grant.ok()).toBe(true);
	await expect(nav.getByRole('link', { name: 'Access' })).toBeVisible({ timeout: 10_000 });
	const revoke = await page.request.delete('/api/v1/access/admins/smoke-member@example.com', { headers });
	expect(revoke.ok()).toBe(true);
	await expect(nav.getByRole('link', { name: 'Access' })).toHaveCount(0, { timeout: 10_000 });
	expect(navigations).toBe(loaded);
	await member.close();
});
