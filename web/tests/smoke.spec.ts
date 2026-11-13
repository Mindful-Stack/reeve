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
	// Lands on Planning, whose page loads no story list (the top bar's untriaged count does; counters filter it out).
	await page.goto('/auth/login?next=/planning');
	await expect(page).toHaveURL(/\/planning$/);
	const session = (await context.cookies()).find((c) => c.name === 'reeve_session');
	expect(session).toBeDefined();
	expect(session!.httpOnly).toBe(true);
	expect(session!.sameSite).toBe('Lax');
	expect(session!.secure).toBe(false); // http on 127.0.0.1
});

test('planning view shows the seeded initiative in Now', async ({ page }) => {
	await page.goto('/planning');
	await expect(page.getByRole('heading', { name: 'Now' })).toBeVisible();
	await expect(page.getByText('Smoke initiative')).toBeVisible();
});

test('Home lists a ready unowned story under Pick next and Start makes it mine', async ({ page }) => {
	const title = `Home smoke ${crypto.randomUUID().slice(0, 8)}`;
	// Its own story with a criterion under the discovery-complete smoke epic (ready), never SMOKE_STORY, which other tests move.
	const s = await post<{ key: string }>(page, '/api/v1/stories', {
		idempotency_key: crypto.randomUUID(), title, kind: 'chore', criteria: ['it starts'], labels: [], epic: EPIC
	});
	// This runs before the tests that add ready stories, so the new story is among the five Pick next shows.
	await page.goto('/');
	const pick = page.getByRole('region', { name: 'Pick next' });
	await pick.getByRole('button', { name: `Start ${s.key}` }).click();
	await expect(page.getByText(`${s.key} is yours now.`)).toBeVisible();
	await expect(page.getByRole('region', { name: 'Under way' })).toContainText(title);
	await expect(pick.getByRole('button', { name: `Start ${s.key}` })).toHaveCount(0);
});

test('board shows ready and blocked stories and the drawer opens by url', async ({ page }) => {
	await page.goto('/board');
	await expect(page.getByTestId(`card-${STORY}`)).toContainText('ready');
	// Blocked (and untriaged) cards sit in the collapsed "Not ready" disclosure.
	await expect(page.locator('details.not-ready > summary')).toContainText(/\b(waits|wait) on decisions$/);
	await page.locator('details.not-ready > summary').click();
	await expect(page.getByTestId(`card-${BLOCKED}`)).toContainText('no criteria');
	await expect(page.getByTestId(`card-${DECISION_STORY}`)).toContainText('decision: no record');
	await page.goto('/board?story=' + STORY);
	await expect(page.getByRole('complementary')).toContainText(STORY);
});

test('the top bar stays at the top while the board scrolls', async ({ page }) => {
	await page.setViewportSize({ width: 1280, height: 300 });
	await page.goto('/board');
	await expect(page.getByTestId(`card-${STORY}`)).toBeVisible(); // the board has loaded and is taller than the viewport
	const bar = page.getByRole('banner');
	await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
	expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(0); // the page really scrolled
	const box = (await bar.boundingBox())!;
	expect(box.y).toBe(0);
	await page.getByRole('button', { name: 'Report', exact: true }).click({ trial: true }); // visible and not covered
});

test('board asks for recently done stories and labels the done column', async ({ page }) => {
	const req = page.waitForRequest(
		(r) => new URL(r.url()).pathname === '/api/v1/stories' && r.method() === 'GET' && new URL(r.url()).searchParams.has('done')
	);
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

test('the card\'s copy button does not start the card\'s drag, and does not make the meta row tall', async ({ page }) => {
	const s = await chore(page, `Smoke copy drag ${crypto.randomUUID().slice(0, 6)}`);
	await page.goto('/board');
	// A new chore is not ready, so its card sits in the collapsed "Not ready" disclosure; dragging it would open the override prompt.
	const openNotReady = async () => {
		const nr = page.locator('details.not-ready');
		if ((await nr.getAttribute('open')) === null) await nr.locator('summary').click();
	};
	await openNotReady();
	const card = page.getByTestId(`card-${s.key}`);
	await expect(card).toBeVisible();
	expect((await card.locator('.meta').boundingBox())!.height).toBeLessThan(24);
	await page.evaluate(() => {
		(window as unknown as { dragStarts: number }).dragStarts = 0;
		document.addEventListener('dragstart', () => (window as unknown as { dragStarts: number }).dragStarts++);
	});
	await card.getByRole('button', { name: `Copy ${s.key}` }).dragTo(page.getByTestId('column-in_progress'));
	expect(await page.evaluate(() => (window as unknown as { dragStarts: number }).dragStarts)).toBe(0); // the browser starts the card's drag from a non-draggable descendant
	await expect(page.getByRole('dialog')).toHaveCount(0); // the override prompt a drop would open
	await page.reload();
	await openNotReady();
	await expect(page.getByTestId('column-todo').getByTestId(`card-${s.key}`)).toBeVisible();
	await expect(page.getByTestId('column-in_progress').getByTestId(`card-${s.key}`)).toHaveCount(0);
});

test('a move made elsewhere reaches an open board over the event stream, without a reload', async ({ page, browser }) => {
	// The board loads once on mount and refetches once 500 ms after the stream opens. Wait for both before the other
	// browser moves the story, so the card can only reach in progress through an `activity` frame: with the stream's
	// `activity` listener removed (or the event renamed on the server) this test fails, since no later refetch happens.
	let loads = 0;
	page.on('response', (r) => {
		const u = new URL(r.url());
		if (u.pathname === '/api/v1/stories' && u.searchParams.get('done') === 'recent') loads++;
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
	// The layout's untriaged count loads the same way (on mount, then after the stream opens): wait for both of those too.
	// It also refetches on any activity entry the stream carries, and other tests share this database, so a third read
	// can land; "at least two" is what the test needs (both answered before the session is revoked), not "exactly two".
	let loads = 0;
	let counts = 0;
	page.on('response', (r) => {
		const u = new URL(r.url());
		if (u.pathname === '/api/v1/stories' && u.searchParams.get('done') === 'recent') loads++;
		if (u.pathname === '/api/v1/stories' && u.searchParams.get('untriaged') === 'true') counts++;
	});
	await page.goto('/board');
	await expect(page.getByText('live', { exact: true })).toBeAttached(); // visually hidden
	await expect.poll(() => loads).toBe(2);
	await expect.poll(() => counts).toBeGreaterThanOrEqual(2);
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

test('the drawer epic picker: typing replaces the label, the active option stays in sight, Enter picks then saves', async ({ page }) => {
	const suffix = crypto.randomUUID().slice(0, 6);
	// Twelve matches overflow the 16rem list.
	for (let i = 0; i < 12; i++) {
		await post(page, '/api/v1/epics', { idempotency_key: crypto.randomUUID(), title: `Smoke picker ${suffix} ${String(i).padStart(2, '0')}` });
	}
	const s = await chore(page, `Smoke picker story ${suffix}`); // under EPIC: the closed picker shows EPIC's label
	await page.goto(`/board?story=${s.key}`);
	const drawer = page.getByRole('complementary');
	await drawer.getByRole('button', { name: 'Edit epic' }).click();
	await expect(drawer.getByRole('combobox', { name: 'Epic' })).toBeFocused(); // focused by script, not by a click
	await page.keyboard.type(`picker ${suffix}`); // keys, not fill(): they must replace the selected label
	const list = drawer.getByRole('listbox', { name: 'Epic' });
	await expect(list.getByRole('option')).toHaveCount(12);
	for (let i = 0; i < 11; i++) await page.keyboard.press('ArrowDown');
	await expect(list.getByRole('option', { name: new RegExp(`${suffix} 11$`) })).toBeInViewport();
	await page.keyboard.press('Enter'); // picks; the list closes
	await expect(list).toBeHidden();
	await page.keyboard.press('Enter'); // closed, Enter is the form's: it saves
	await expect(drawer.getByRole('link', { name: new RegExp(`^\\S+ Smoke picker ${suffix} 11$`) })).toBeVisible();
	// Cancel sits beside the open list: the press blurs the input and closes the list, which moves nothing, so the click lands.
	await drawer.getByRole('button', { name: 'Edit epic' }).click();
	await page.keyboard.press('ArrowDown');
	await expect(list).toBeVisible();
	await drawer.getByRole('button', { name: 'Cancel', exact: true }).click();
	await expect(drawer.getByRole('combobox', { name: 'Epic' })).toBeHidden();
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
	await expect(row.getByRole('button', { name: 'Holds 1 story: Smoke one-way decision' })).toBeVisible();

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
	await page.goto('/planning');
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
	await page.getByRole('form', { name: 'Epic details' }).getByRole('combobox', { name: 'Initiative' }).click();
	await page.getByRole('option', { name: 'No initiative', exact: true }).click();
	await page.getByRole('form', { name: 'Epic details' }).getByRole('button', { name: 'Save details' }).click();
	await expect(page.getByText('This epic belongs to no initiative.')).toBeVisible();
	await page.getByRole('button', { name: 'Choose an initiative' }).click();
	await page.getByRole('form', { name: 'Epic details' }).getByRole('combobox', { name: 'Initiative' }).fill(iniKey);
	await page.getByRole('option', { name: new RegExp('^' + iniKey + '\\b') }).click();
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
	await expect(indexForm.getByRole('combobox', { name: 'Initiative' })).toHaveValue('No initiative'); // the closed picker shows the empty choice
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
	// Change epic: the dialog focuses its picker; the open list floats past the dialog's edge and is not clipped by it.
	await post(page, '/api/v1/epics', { idempotency_key: crypto.randomUUID(), title: 'Smoke change epic target' });
	await page.reload();
	await page.getByRole('button', { name: `Actions for ${a.key}`, exact: true }).click();
	await page.getByRole('menuitem', { name: 'Change epic' }).click();
	const changeEpic = page.getByRole('dialog');
	await expect(changeEpic.getByRole('combobox', { name: 'Epic' })).toBeFocused();
	await page.keyboard.press('ArrowDown');
	// ratio 1: the whole list box intersects the viewport, after the clipping of every scrolling ancestor (the dialog).
	await expect(changeEpic.getByRole('listbox', { name: 'Epic' })).toBeInViewport({ ratio: 1 });
	await page.keyboard.press('Escape'); // the list only
	await expect(changeEpic.getByRole('listbox')).toBeHidden();
	await expect(changeEpic).toBeVisible();
	await page.keyboard.press('Escape'); // now the dialog
	await expect(changeEpic).toBeHidden();
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
	await expect(page.getByRole('heading', { level: 1, name: 'Home' })).toBeVisible();
	// The seed makes the smoke user the decider of a decision that holds a story, so the section and the flag show.
	const decisions = page.getByRole('region', { name: 'Decisions you decide' });
	await expect(decisions).toBeVisible();
	// The seeded decision is not needed this sprint or next (no goal links it), so Home counts it rather than listing it.
	await expect(decisions.getByText(/more beyond the next sprint or not blocking yet/)).toBeVisible();
	await expect(page.getByText(/stor(y|ies) held by a decision/)).toBeVisible();
	await page.goto('/planning');
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
			await page.locator('details.not-ready > summary').click();
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

// The Add decision form (GRA-636): the sprint-end shortcut row is offered and its buttons lie inside the form's box.
for (const width of [320, 1280]) {
	test(`the Add decision form keeps its sprint-end shortcuts inside its box at ${width}px`, async ({ page }) => {
		const ini = await post<{ key: string }>(page, '/api/v1/initiatives', {
			idempotency_key: crypto.randomUUID(),
			title: `Decision form smoke ${crypto.randomUUID().slice(0, 8)}`,
			horizon: 'Now'
		});
		// The shortcuts need a sprint end: open a sprint (a goal creates the draft when there is none) and close it after.
		type Listed = {
			active: { sprint: { id: number; version: number; goals: { id: number }[] } } | null;
			draft: { id: number; version: number } | null;
		};
		const listed = async () => ((await (await page.request.get('/api/v1/sprints', { headers })).json()) as { value: Listed }).value;
		if ((await listed()).draft == null) {
			await post(page, '/api/v1/goals', { idempotency_key: crypto.randomUUID(), outcome: `Decision form goal ${crypto.randomUUID().slice(0, 8)}`, check: 'the shortcuts show' });
		}
		const draft = (await listed()).draft!;
		await post(page, `/api/v1/sprints/${draft.id}/open`, { version: draft.version });
		const sprint = (await listed()).active!.sprint;
		try {
			await page.setViewportSize({ width, height: 900 });
			await page.goto(`/initiatives/${ini.key}`);
			await page.getByRole('button', { name: '+ Add decision' }).click();
			const form = page.getByRole('dialog').getByRole('form', { name: 'Add decision' });
			await expect(form).toBeVisible();
			const group = form.getByRole('group', { name: 'Due date shortcuts' });
			await expect(group).toBeVisible();
			const box = (await form.boundingBox())!;
			const buttons = await group.getByRole('button').all();
			expect(buttons.length).toBeGreaterThan(0);
			for (const el of buttons) {
				const b = (await el.boundingBox())!;
				expect(b.x + b.width, `${await el.innerText()} right edge`).toBeLessThanOrEqual(box.x + box.width + 0.5);
				expect(b.x).toBeGreaterThanOrEqual(box.x - 0.5);
			}
		} finally {
			await post(page, `/api/v1/sprints/${sprint.id}/close`, {
				version: sprint.version,
				verdicts: sprint.goals.map((g) => ({ goal_id: g.id, verdict: 'missed' }))
			});
		}
	});
}

// A decision's inline edit form (GRA-636): Title takes the form's width, no field is cut or spills, and the page does
// not scroll sideways at a phone width, even with an empty list.
for (const width of [320, 1280]) {
	test(`a decision's edit form fits its row at ${width}px`, async ({ page }) => {
		const ini = await post<{ key: string }>(page, '/api/v1/initiatives', {
			idempotency_key: crypto.randomUUID(),
			title: `Edit form smoke ${crypto.randomUUID().slice(0, 8)}`,
			horizon: 'Now'
		});
		await page.setViewportSize({ width, height: 900 });
		await page.goto(`/initiatives/${ini.key}`);
		// Measure the loaded page, not its "Loading…" frame.
		await expect(page.getByRole('button', { name: '+ Add decision' })).toBeVisible();
		await expect(page.locator('#decisions').getByText('Nothing blocking')).toBeVisible();
		expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), 'no page scroll with no decisions').toBe(true);
		const title = 'Portal ADR (one-way): the org profile carries the fields and drives a hidden watch';
		await post(page, '/api/v1/decisions', {
			idempotency_key: crypto.randomUUID(),
			on: ini.key,
			title,
			decider: 'daniel.thyselius@grantigo.com'
		});
		await page.goto(`/initiatives/${ini.key}`);
		const row = page.getByRole('listitem').filter({ hasText: title });
		await row.getByRole('button', { name: /^Actions for Portal ADR/ }).click();
		await page.getByRole('menuitem', { name: 'Edit' }).click();
		const form = row.getByRole('form', { name: /^Edit decision / });
		await expect(form).toBeVisible();
		const box = (await form.boundingBox())!;
		for (const field of ['title', 'decider', 'due', 'reversibility']) {
			const b = (await form.locator(`[data-field="${field}"]`).boundingBox())!;
			expect(b.x + b.width, `${field} right edge`).toBeLessThanOrEqual(box.x + box.width + 0.5);
			expect(b.x, `${field} left edge`).toBeGreaterThanOrEqual(box.x - 0.5);
			if (field === 'title') expect(b.width, 'title width').toBeGreaterThanOrEqual(box.width * 0.6);
		}
		// The title is longer than a phone's line: only the wide layout shows all of it.
		if (width >= 1280) {
			for (const field of ['title', 'decider']) {
				const el = form.locator(`[data-field="${field}"]`);
				const [wide, shown] = await el.evaluate((e: HTMLInputElement) => [e.scrollWidth, e.clientWidth]);
				expect(wide, `${field} shows its full value`).toBeLessThanOrEqual(shown + 1);
			}
		}
		expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), 'no page scroll').toBe(true);
	});
}

test('the header goal picker links an initiative and, after navigating by click, one of its epics', async ({ page }) => {
	const outcome = `Picker smoke ${crypto.randomUUID().slice(0, 8)}`;
	await page.goto('/planning');
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
	await page.goto('/planning');
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

test('report a bug from the board: it lands in triage, and a similar story can be checked without losing the draft', async ({ page }) => {
	const suffix = crypto.randomUUID().slice(0, 6);
	const title = `Smoke export PDF is blank ${suffix}`;
	const existing = await chore(page, title); // an existing item to be found as similar (under SMOKE_EPIC)
	await page.goto('/board');
	const dialog = page.getByRole('dialog', { name: 'Report something' });
	await page.getByRole('button', { name: 'Report', exact: true }).click();
	await page.mouse.click(5, 5); // the backdrop closes an untouched dialog
	await expect(dialog).toBeHidden();
	await page.getByRole('button', { name: 'Report', exact: true }).click();
	await dialog.getByLabel('Title').fill(`PDF export shows a blank page ${suffix}`);
	await page.mouse.click(5, 5); // the backdrop: ignored once something is typed
	await expect(dialog).toBeVisible();
	await page.keyboard.press('Escape'); // closes and keeps the draft
	await expect(dialog).toBeHidden();
	await page.getByRole('banner').getByRole('button', { name: 'Resume report' }).click();
	await expect(dialog.getByLabel('Title')).toHaveValue(`PDF export shows a blank page ${suffix}`);
	const hit = dialog.locator(`[data-hit="${existing.key}"]`);
	await expect(hit).toContainText(title); // "Already reported?" found it
	await hit.click();
	await expect(dialog).toBeHidden();
	const drawer = page.getByRole('complementary');
	await expect(drawer).toContainText(title); // the drawer is usable, not inert
	await page.getByRole('banner').getByRole('button', { name: 'Resume report' }).click(); // the drawer opens under the bar
	await expect(dialog.getByLabel('Title')).toHaveValue(new RegExp(`PDF export shows a blank page ${suffix}`));
	await dialog.getByLabel("Something's broken").check();
	await dialog.getByRole('button', { name: 'Report', exact: true }).click();
	const notice = page.getByRole('status').filter({ hasText: 'Filed bug' });
	await expect(notice).toContainText('waiting for the team to place it');
	const key = (await notice.textContent())!.match(/[A-Z]+-\d+/)![0];
	await page.goto('/board?untriaged=1');
	await expect(page.getByRole('checkbox', { name: 'Untriaged' })).toBeChecked();
	await page.locator('details.not-ready > summary').click();
	await expect(page.getByTestId(`card-${key}`)).toContainText('untriaged');
	// The filter really filters: the similar story sits under an epic, so it is not listed here.
	await expect(page.getByTestId(`card-${existing.key}`)).toHaveCount(0);
});

test('the story drawer header fits at 320px with a draft: Close stays on screen and the bar\'s Resume report stays usable', async ({ page }) => {
	await page.setViewportSize({ width: 320, height: 640 });
	const s = await chore(page, `Smoke drawer header ${crypto.randomUUID().slice(0, 6)}`);
	await page.goto('/board');
	await page.getByRole('button', { name: 'Report', exact: true }).click();
	const dialog = page.getByRole('dialog', { name: 'Report something' });
	await dialog.getByLabel('Title').fill('A draft that outlives the dialog');
	await page.keyboard.press('Escape');
	await expect(dialog).toBeHidden();
	await page.goto(`/board?story=${s.key}`);
	const drawer = page.getByRole('complementary');
	const resume = page.getByRole('banner').getByRole('button', { name: 'Resume report' });
	const close = drawer.getByRole('button', { name: `Close story ${s.key}` });
	await expect(resume).toBeVisible();
	await expect(close).toBeVisible();
	await close.click({ trial: true }); // not covered by the sticky bar
	await resume.click({ trial: true }); // the bar's button is usable while the drawer is open
	const c = (await close.boundingBox())!;
	expect(c.x + c.width).toBeLessThanOrEqual(320);
	// The key and its glyph stay on one line; only the chips after it may wrap.
	const key = drawer.locator('h2 .key');
	expect((await key.boundingBox())!.height).toBeLessThan(30);
	// Squeeze the heading line: the key must not break even when it has to overflow.
	await drawer.locator('.heading-line').evaluate((el) => ((el as HTMLElement).style.width = '40px'));
	expect((await key.boundingBox())!.height).toBeLessThan(30);
	await expect(drawer.getByRole('button', { name: `Copy ${s.key}` })).toBeVisible();
});

for (const width of [1280, 360]) {
	test(`drawers open under the top bar at ${width}px`, async ({ page }) => {
		await page.setViewportSize({ width, height: 700 });
		const s = await chore(page, `Smoke drawer under bar ${crypto.randomUUID().slice(0, 6)}`);
		const d = await post<{ id: number }>(page, '/api/v1/decisions', {
			idempotency_key: crypto.randomUUID(), on: s.key, title: 'Smoke decision under the bar', decider: 'smoke@example.com'
		});
		const banner = page.getByRole('banner');
		const under = async (drawer: Locator) => {
			await expect(drawer).toBeVisible();
			const bar = (await banner.boundingBox())!;
			const box = (await drawer.boundingBox())!;
			expect(box.y).toBeGreaterThanOrEqual(bar.y + bar.height - 1);
			expect(box.y).toBeLessThanOrEqual(bar.y + bar.height + 1); // no gap
			await close.click({ trial: true }); // fails if the bar covers the drawer
		};
		let close = page.getByRole('button', { name: `Close story ${s.key}` });
		await page.goto(`/board?story=${s.key}`);
		await under(page.getByRole('complementary'));
		close = page.getByRole('button', { name: `Close decision #${d.id}` });
		await page.goto(`/decisions?decision=${d.id}`);
		await under(page.getByTestId('decision-drawer'));
	});
}

for (const width of [1280, 360]) {
	test(`the account menu opens from the top bar at ${width}px and stays open`, async ({ page }) => {
		await page.setViewportSize({ width, height: 700 });
		await page.goto('/board');
		const trigger = page.getByRole('banner').getByRole('button', { name: /^Account:/ });
		await trigger.click();
		await expect(page.getByRole('menuitem', { name: 'Sign out' })).toBeVisible();
		await page.waitForTimeout(300); // a menu that closes itself after opening would be gone by now
		await expect(page.getByRole('menuitem', { name: 'Sign out' })).toBeVisible();
		await page.keyboard.press('Escape');
		await expect(page.getByRole('menuitem', { name: 'Sign out' })).toHaveCount(0);
	});
}

for (const [width, height, phone] of [[320, 640, true], [720, 800, true], [1280, 800, false]] as const) {
	test(`the header fits at ${width}px: no horizontal scroll, Report reachable, search usable`, async ({ page }) => {
		await page.setViewportSize({ width, height });
		await chore(page, `Header fit untriaged ${crypto.randomUUID().slice(0, 6)}`, null); // the bar carries an untriaged count
		await page.goto('/board');
		await expect(page.getByRole('checkbox', { name: 'Untriaged' })).toBeVisible(); // the board has rendered
		await expect(page.locator('header.top-bar a.untriaged')).toBeVisible();
		const report = page.getByRole('button', { name: 'Report', exact: true });
		await expect(report).toBeVisible();
		const search = page.getByRole('search', { name: 'Site search' }).getByRole('searchbox');
		await expect(search).toBeVisible();
		const m = await page.evaluate(() => ({ scroll: document.documentElement.scrollWidth, client: document.documentElement.clientWidth }));
		expect(m.scroll).toBeLessThanOrEqual(m.client);
		expect((await search.boundingBox())!.width).toBeGreaterThanOrEqual(80);
		// Below 720px the word is clipped to 1x1 (still "visible" to Playwright), so measure it.
		const wordWidth = (await report.locator('.word').boundingBox())?.width ?? 0;
		if (phone) expect(wordWidth).toBeLessThanOrEqual(1);
		else expect(wordWidth).toBeGreaterThan(20);
		if (phone) await expect(report.locator('.plus')).toBeVisible();
		else await expect(report.locator('.plus')).toBeHidden();
		await report.click();
		await expect(page.getByRole('dialog', { name: 'Report something' })).toBeVisible();
	});
}

// A 4×3 PNG, small enough to inline; every image assertion waits for it to really decode (a broken image is visible too).
const PNG = Buffer.from(
	'iVBORw0KGgoAAAANSUhEUgAAAAQAAAADAQMAAACOOjyFAAAAA1BMVEUzZswgIiSpAAAAC0lEQVQI12NgAAEAAAYAAWb0yWwAAAAASUVORK5CYII=',
	'base64'
);
const loaded = (img: Locator) => expect.poll(() => img.evaluate((i: HTMLImageElement) => i.complete && i.naturalWidth > 0), { timeout: 10_000 }).toBe(true);

test('a screenshot added to a report shows in the story, a pasted one in a comment, and a deleted one becomes unavailable', async ({ page }) => {
	const suffix = crypto.randomUUID().slice(0, 6);
	await page.goto('/board');
	await page.getByRole('button', { name: 'Report', exact: true }).click();
	const dialog = page.getByRole('dialog', { name: 'Report something' });
	await dialog.getByLabel('Title').fill(`Screenshot smoke ${suffix}`);
	await dialog.getByLabel("Something's broken").check();
	await dialog.locator('input[type=file]').setInputFiles({ name: 'report.png', mimeType: 'image/png', buffer: PNG });
	await expect(dialog.locator('.uploads li')).toContainText('added');
	await dialog.getByRole('button', { name: 'Report', exact: true }).click();
	const notice = page.getByRole('status').filter({ hasText: 'Filed bug' });
	const key = (await notice.textContent())!.match(/[A-Z]+-\d+/)![0];

	await page.goto(`/board?story=${key}`);
	const drawer = page.getByRole('complementary');
	await loaded(drawer.locator('.brief img'));
	await drawer.getByText('Attachments · 1').click();
	await loaded(drawer.locator('.attachments img'));

	// A paste of an image-only clipboard into the comment box (Chromium has DataTransfer and ClipboardEvent).
	const box = drawer.getByLabel(`comment on ${key}`);
	await box.click();
	await box.evaluate((el, b64) => {
		const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
		const dt = new DataTransfer();
		dt.items.add(new File([bytes], 'image.png', { type: 'image/png' }));
		el.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
	}, PNG.toString('base64'));
	await expect(box).toHaveValue(/!\[screenshot-[\d-]+\]\(\/api\/v1\/attachments\/[0-9a-f-]+\)/);
	await drawer.getByRole('button', { name: 'Comment', exact: true }).click();
	const commentImg = drawer.locator('.comment-body img');
	await loaded(commentImg);
	await expect(drawer.getByText('Attachments · 2')).toBeVisible();

	// Delete the comment's image from the disclosure: the shown image becomes "image unavailable" without a reload.
	page.once('dialog', (d) => d.accept());
	await drawer.getByRole('button', { name: /^Delete attachment screenshot-/ }).click();
	await expect(drawer.locator('.comment-body .image-unavailable')).toHaveText('image unavailable');
	await expect(drawer.getByText('Attachments · 1')).toBeVisible();

	// A file that is not an image is refused before it is uploaded.
	await drawer.locator('form:has(textarea[aria-label^="comment"]) input[type=file]').setInputFiles({ name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('hi') });
	await expect(drawer.locator('.uploads')).toContainText('only PNG, JPEG and WebP images can be attached');
});

test('the header box filters the open backlog by relevance, links to search all, and jumps to a key', async ({ page }) => {
	const word = `zebra${Date.now()}`;
	const story = (title: string, description: string) =>
		post<{ key: string }>(page, '/api/v1/stories', { idempotency_key: crypto.randomUUID(), title, description, kind: 'chore', epic: EPIC, criteria: [], labels: [] });
	// Ranked first, but only its description matches; the title match ranks below it and sorts above it by relevance.
	const byDescription = await story('Smoke relevance first', `Mentions ${word} in passing`);
	const byTitle = await story(`Smoke ${word} in the title`, '');
	await page.goto('/board');
	const box = page.getByRole('search', { name: 'Site search' }).getByRole('searchbox');
	await box.fill(word);
	await box.press('Enter');
	await expect(page).toHaveURL(`/backlog?q=${word}&state=open&view=relevance`);
	const rows = page.locator('ol.rows li[data-key]');
	await expect(rows).toHaveCount(2);
	expect(await rows.evaluateAll((li) => li.map((x) => x.getAttribute('data-key')))).toEqual([byTitle.key, byDescription.key]);
	await expect(rows.first().locator('mark')).toHaveText(word);
	await expect(rows.nth(1)).toContainText('matched description');
	await expect(box).toHaveValue(word);
	// Ranking is offered only outside relevance.
	await page.getByRole('button', { name: `Actions for ${byTitle.key}`, exact: true }).click();
	await expect(page.getByRole('menuitem', { name: 'Move to in progress' })).toBeVisible();
	await expect(page.getByRole('menuitem', { name: 'Move up' })).toHaveCount(0);
	await page.keyboard.press('Escape');
	// Not here? Search all items: /search?q=, where a key typed in the header box opens its drawer over the search.
	await page.getByRole('link', { name: /^Search all items, including epics, initiatives and done work/ }).click();
	await expect(page).toHaveURL(`/search?q=${word}`);
	await expect(page.getByRole('link', { name: new RegExp(`${byTitle.key}.*${word}`) })).toBeVisible();
	await box.fill(byDescription.key);
	await box.press('Enter');
	await expect(page).toHaveURL(`/search?q=${word}&story=${byDescription.key}`);
	await expect(page.getByRole('complementary')).toContainText('Smoke relevance first');
});

for (const width of [320, 720, 1280]) {
	test(`the backlog's filter bar fits at ${width}px with the filters open: no horizontal scroll`, async ({ page }) => {
		await page.setViewportSize({ width, height: 800 });
		// The widest options the pickers can hold: an epic with an 80-character title (the Epic picker) and a long label
		// on a todo story (the Label picker).
		const title = `Smoke epic at ${width}px with a title long enough to fill eighty characters`.padEnd(80, '.');
		expect(title).toHaveLength(80);
		await post(page, '/api/v1/epics', { idempotency_key: crypto.randomUUID(), title });
		await post(page, '/api/v1/stories', {
			idempotency_key: crypto.randomUUID(),
			title: `Smoke filter bar at ${width}px`,
			kind: 'chore',
			epic: EPIC,
			criteria: [],
			labels: ['smoke-a-label-long-enough-to-name-a-whole-area-of-the-product-and-then-some-more']
		});
		await page.goto('/backlog?q=smoke&kind=chore&state=open');
		await expect(page.getByRole('group', { name: 'Filters' })).toBeVisible(); // open on load: a filter is set
		await expect(page.getByRole('button', { name: 'Filters (1)' })).toHaveAttribute('aria-expanded', 'true');
		await expect(page.getByRole('button', { name: /^Remove the text filter/ })).toBeVisible();
		await expect(page.locator('ol.rows li[data-key]').first()).toBeVisible();
		const panel = page.getByRole('group', { name: 'Filters' });
		await panel.getByRole('combobox', { name: 'Label' }).click();
		await page.keyboard.type('smoke-a-label');
		await expect(panel.getByRole('option', { name: 'smoke-a-label-long-enough' })).toHaveCount(1);
		await page.keyboard.press('Escape');
		// The open epic list, with the widest option, stays open while the measurement runs.
		await panel.getByRole('combobox', { name: 'Epic' }).click();
		await page.keyboard.type(title.slice(0, 40));
		await expect(panel.getByRole('option', { name: title })).toHaveCount(1);
		const m = await page.evaluate(() => ({
			scroll: document.documentElement.scrollWidth,
			client: document.documentElement.clientWidth,
			// What sticks out, for the failure message.
			wide: [...document.querySelectorAll('main *')].filter((e) => e.getBoundingClientRect().right > document.documentElement.clientWidth + 0.5).map((e) => e.tagName + '.' + e.className)
		}));
		expect(m.scroll, `wider than the page: ${m.wide.join(', ')}`).toBeLessThanOrEqual(m.client);
	});
}

test('the board banner counts a decision needed this sprint, links to its lane, and a row title opens the decision drawer', async ({ page }) => {
	const tag = crypto.randomUUID().slice(0, 8);
	const title = `Banner smoke decision ${tag}`;
	const epic = await post<{ key: string }>(page, '/api/v1/epics', { idempotency_key: crypto.randomUUID(), title: `Banner smoke epic ${tag}` });
	await post(page, '/api/v1/decisions', { idempotency_key: crypto.randomUUID(), on: epic.key, title, decider: 'smoke@example.com' });
	await post(page, '/api/v1/stories', {
		idempotency_key: crypto.randomUUID(), title: `Banner smoke story ${tag}`, kind: 'chore', criteria: ['it is decided'], labels: [], epic: epic.key
	});

	// A goal over the epic in an open sprint: the decision is needed this sprint. This test opens its own sprint (after the
	// sprint cycle, which needs none active) and closes it at the end.
	const { goal } = await post<{ goal: { id: number } }>(page, '/api/v1/goals', {
		idempotency_key: crypto.randomUUID(), outcome: `Banner smoke goal ${tag}`, check: 'the banner shows it'
	});
	await post(page, `/api/v1/goals/${goal.id}/links`, { items: [epic.key] });
	const listed = async () =>
		((await (await page.request.get('/api/v1/sprints', { headers })).json()) as {
			value: { active: { sprint: { id: number; version: number; goals: { id: number }[] } } | null; draft: { id: number; version: number } | null };
		}).value;
	const draft = (await listed()).draft!;
	await post(page, `/api/v1/sprints/${draft.id}/open`, { version: draft.version });
	const sprint = (await listed()).active!.sprint;

	try {
		await page.goto('/board');
		const banner = page.getByRole('navigation', { name: 'Decisions needing attention' });
		// Earlier tests link the draft goal to the initiative, so other decisions are needed this sprint too: the count is at least ours.
		const link = banner.getByRole('link', { name: /^\d+ decisions? needed this sprint$/ });
		await expect(link).toBeVisible();
		await link.click();
		await expect(page).toHaveURL(/\/decisions#this-sprint$/);
		await expect(page.locator('h2#this-sprint')).toBeVisible();

		await page.getByRole('link', { name: title, exact: true }).click();
		const drawer = page.getByTestId('decision-drawer');
		await expect(drawer).toBeVisible();
		await expect(drawer.getByRole('heading', { level: 2 }).locator('.h-title')).toHaveText(title);
		await expect(page).toHaveURL(/[?&]decision=\d+/);
	} finally {
		await post(page, `/api/v1/sprints/${sprint.id}/close`, {
			version: sprint.version,
			verdicts: sprint.goals.map((g) => ({ goal_id: g.id, verdict: 'missed' }))
		});
	}
});

test("a story promised by the end of this sprint shows its chip after a reload, and clearing it shows its epic's promise again", async ({ page }) => {
	const tag = crypto.randomUUID().slice(0, 8);
	const epic = await post<{ key: string }>(page, '/api/v1/epics', { idempotency_key: crypto.randomUUID(), title: `Promise smoke epic ${tag}` });
	const story = await post<{ key: string }>(page, '/api/v1/stories', {
		idempotency_key: crypto.randomUUID(), title: `Promise smoke story ${tag}`, kind: 'chore', criteria: ['it is promised'], labels: [], epic: epic.key
	});

	// An open sprint and a next draft, with planned ends for the shortcuts. The seed has no sprint (earlier tests need
	// none), so this test opens its own and closes it at the end; a goal creates the draft when there is none.
	type Listed = {
		active: { sprint: { id: number; version: number; ends_on: string; goals: { id: number }[] } } | null;
		draft: { id: number; version: number; ends_on: string } | null;
	};
	const listed = async () => ((await (await page.request.get('/api/v1/sprints', { headers })).json()) as { value: Listed }).value;
	if ((await listed()).draft == null) {
		await post(page, '/api/v1/goals', { idempotency_key: crypto.randomUUID(), outcome: `Promise smoke goal ${tag}`, check: 'the chip shows it' });
	}
	const draft = (await listed()).draft!;
	await post(page, `/api/v1/sprints/${draft.id}/open`, { version: draft.version });
	const { active, draft: next } = await listed();
	const sprint = active!.sprint;

	try {
		// The epic is promised by the next sprint's end, after the active sprint's: the story inherits it.
		expect(next!.ends_on > sprint.ends_on).toBe(true);
		await post(page, `/api/v1/epics/${epic.key}/promise`, { shortcut: 'next_sprint', to: 'the customer' });
		const day = (iso: string) => {
			const [, m, d] = iso.split('-').map(Number);
			return `${d} ${['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][m - 1]}`;
		};

		// The story is not ready (its epic's discovery is open), so its card sits in the Not ready disclosure: `notready=1`
		// opens it, and the URL keeps it open across the reload.
		await page.goto(`/board?story=${story.key}&notready=1`);
		const drawer = page.getByRole('complementary');
		const line = drawer.getByTestId('promise-line');
		await expect(line).toContainText(`Promised by ${day(next!.ends_on)} ${next!.ends_on.slice(0, 4)} via epic ${epic.key}`);

		await drawer.getByRole('button', { name: `More actions for ${story.key}` }).click();
		await page.getByRole('menuitem', { name: 'Promise by…' }).click();
		const dialog = page.getByRole('dialog', { name: `Promise ${story.key} by…` });
		await dialog.getByRole('button', { name: `End of this sprint (${day(sprint.ends_on)})` }).click();
		await dialog.getByRole('button', { name: 'Promise', exact: true }).click();
		await expect(dialog).toBeHidden();

		const chip = page.getByTestId(`card-${story.key}`).locator('.promise-chip');
		await expect(chip).toBeVisible();
		await expect(chip).toHaveText(`Promised by ${day(sprint.ends_on)} · this sprint`);

		await page.reload();
		await expect(page.locator('details.not-ready')).toHaveAttribute('open', '');
		await expect(chip).toBeVisible();
		await expect(chip).toHaveText(`Promised by ${day(sprint.ends_on)} · this sprint`);

		// The recipient's limit counts characters as the server does (code points), in a real browser with real input
		// events: 201 emoji (supplementary characters, 402 UTF-16 units) are refused in the dialog without a request, and
		// 200 reach the server whole.
		const writes: string[] = [];
		const isWrite = (r: { method(): string; url(): string }) =>
			r.method() === 'POST' && new URL(r.url()).pathname === `/api/v1/stories/${story.key}/promise`;
		page.on('request', (r) => {
			if (isWrite(r)) writes.push(r.postData() ?? '');
		});
		await page.getByRole('complementary').getByRole('button', { name: `More actions for ${story.key}` }).click();
		await page.getByRole('menuitem', { name: 'Change promise…' }).click();
		const change = page.getByRole('dialog', { name: `Change the promise on ${story.key}` });
		const to = change.locator('input[name="to"]');
		await to.clear();
		await to.pressSequentially('😀'.repeat(201));
		expect([...(await to.inputValue())].length).toBe(201);
		await change.getByRole('button', { name: 'Save', exact: true }).click();
		await expect(change).toBeVisible();
		expect(await to.evaluate((el: HTMLInputElement) => el.validity.customError)).toBe(true);
		expect(writes).toEqual([]);
		await to.press('Backspace');
		expect([...(await to.inputValue())].length).toBe(200);
		const sent = page.waitForRequest(isWrite);
		await change.getByRole('button', { name: 'Save', exact: true }).click();
		const body = (await sent).postDataJSON() as { to?: string };
		expect([...(body.to ?? '')].length).toBe(200);
		expect(body.to).toBe('😀'.repeat(200));
		await expect(change).toBeHidden();
		expect(writes).toHaveLength(1);

		await page.getByRole('complementary').getByRole('button', { name: `More actions for ${story.key}` }).click();
		await page.getByRole('menuitem', { name: 'Clear promise' }).click();
		await expect(chip).toBeVisible();
		await expect(chip).toHaveText(`Promised by ${day(next!.ends_on)} · next sprint · via ${epic.key}`);
		await expect(page.getByRole('complementary').getByTestId('promise-line')).toContainText(`via epic ${epic.key}`);
	} finally {
		await post(page, `/api/v1/sprints/${sprint.id}/close`, {
			version: sprint.version,
			verdicts: sprint.goals.map((g) => ({ goal_id: g.id, verdict: 'missed' }))
		});
	}
});

test('triage from the keyboard: the count, place with criteria, undo, a phone, and drop', async ({ page }) => {
	const suffix = crypto.randomUUID().slice(0, 6);
	// a: no epic (untriaged), with a long description for the phone check.
	const a = await post<{ key: string }>(page, '/api/v1/stories', {
		idempotency_key: crypto.randomUUID(),
		title: `Triage smoke export blank ${suffix}`,
		kind: 'bug',
		epic: null,
		criteria: [],
		labels: [],
		description: 'Long description. '.repeat(200)
	});
	const b = await chore(page, `Triage smoke search accents ${suffix}`, null);
	await page.goto('/board');
	const count = page.getByRole('banner').getByRole('link', { name: /untriaged$/ });
	await expect(count).toBeVisible();
	const before = Number((await count.textContent())!.split(' ')[0]);
	await count.click();
	await expect(page).toHaveURL(/\/triage$/);
	const list = page.getByRole('listbox', { name: 'Untriaged' });
	const option = (key: string) => list.getByRole('option', { name: new RegExp(`^${key}\\b`) });
	const status = page.getByTestId('triage-status');
	await expect(list).toBeFocused();
	// Go to item a (other untriaged items may exist from earlier tests): click its option, then use only the keyboard.
	await option(a.key).click();
	await page.keyboard.press('e');
	await page.keyboard.type(EPIC); // typed over the Picker's selected label
	await page.keyboard.press('Enter'); // picks; focus moves on to Criteria
	await expect(page.locator('textarea.criteria')).toBeFocused();
	await page.keyboard.type('The PDF shows the page');
	await page.keyboard.press('Control+Enter');
	await expect(status).toHaveText(`Placed ${a.key} under ${EPIC}.`);
	await expect(option(a.key)).toHaveCount(0);
	await expect(count).toHaveText(`${before - 1} untriaged`, { timeout: 10_000 });
	// Undo brings it back, unplaced, with the typed criterion.
	await page.keyboard.press('u');
	await expect(option(a.key)).toHaveCount(1);
	await expect(page.locator('textarea.criteria')).toHaveValue('The PDF shows the page');
	// ... and the server has it unplaced again: the bar's count (a live refetch) is back to what it was.
	await expect(count).toHaveText(`${before} untriaged`, { timeout: 10_000 });
	// A phone, on a known item with a long description: no horizontal scroll, and Tab from Criteria reaches the buttons.
	await page.setViewportSize({ width: 360, height: 740 });
	expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
	await page.locator('textarea.criteria').focus();
	for (const name of [/^Place/, /^Drop/, /^Skip/]) {
		await page.keyboard.press('Tab');
		await expect(page.getByRole('button', { name })).toBeFocused();
		await expect(page.getByRole('button', { name })).toBeInViewport();
	}
	// Armed, the button reads "Confirm drop" and the sentence is in the status line.
	await page.getByRole('button', { name: /^Drop/ }).click();
	await expect(status).toHaveText(`Drop ${a.key}? Press d or click again.`);
	await expect(page.getByRole('button', { name: /^Confirm drop/ })).toBeVisible();
	await page.keyboard.press('Escape'); // cancels the drop
	await expect(status).toHaveText('Drop cancelled.');
	await page.setViewportSize({ width: 1280, height: 720 });
	// Place it again.
	await page.keyboard.press('e');
	await page.keyboard.type(EPIC);
	await page.keyboard.press('Enter');
	await page.keyboard.press('Control+Enter');
	await expect(status).toHaveText(`Placed ${a.key} under ${EPIC}.`);
	// Drop b with d d.
	await option(b.key).click();
	await page.keyboard.press('d');
	await expect(status).toHaveText(`Drop ${b.key}? Press d or click again.`);
	await page.keyboard.press('d');
	await expect(status).toHaveText(`Dropped ${b.key}.`);
	await expect(option(b.key)).toHaveCount(0);
	const dropped = await page.request.get(`/api/v1/stories/${b.key}`);
	expect((await dropped.json()).value).toMatchObject({ delivery: 'done', resolution: 'dropped' });
	// Undo the drop: back in the queue, todo again.
	await list.focus();
	await page.keyboard.press('u');
	await expect(status).toHaveText(`Undone: ${b.key} is back in todo.`);
	await expect(option(b.key)).toHaveCount(1);
	const back = await page.request.get(`/api/v1/stories/${b.key}`);
	expect((await back.json()).value).toMatchObject({ delivery: 'todo' });
});
