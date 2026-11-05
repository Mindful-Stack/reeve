// web/src/lib/goalPicker.svelte.ts: the header goal picker's state, kept outside the routes so linking mode survives
// navigation (a reload ends it). Only the draft's and the active sprint's goals are ever offered: a closed sprint's
// links never change (kb/ADR-0014 R6). Linking writes only goal links, never a move (R4).
import { api, unwrap } from '#lib/api/client.ts';
import type { Goal, Sprint, Sprints } from '#lib/api/types.ts';
import { errorMessage } from '#lib/format.ts';

export type GoalGroup = { label: string; sprint: Sprint };

/** The sprints whose goals can be picked, named as on /sprint: the draft first (goal setting is for the next sprint),
 *  then the active one (so opening the draft mid-meeting does not end linking). */
export function pickable(s: Sprints | null): GoalGroup[] {
	if (!s) return [];
	const out: GoalGroup[] = [];
	if (s.draft) out.push({ label: 'Next sprint', sprint: s.draft });
	if (s.active) out.push({ label: 'Active sprint', sprint: s.active.sprint });
	return out;
}

/** The pickable goal with `id`, or null (none picked, removed, or its sprint closed). */
export function findGoal(s: Sprints | null, id: number | null): Goal | null {
	if (id === null) return null;
	for (const g of pickable(s)) {
		const hit = g.sprint.goals.find((x) => x.id === id);
		if (hit) return hit;
	}
	return null;
}

export const picker = $state<{
	/** The last `GET /sprints` answer; null until the picker first opens. */
	sprints: Sprints | null;
	/** The goal being linked to, or null when not linking. */
	goalId: number | null;
	/** A link write is in flight: every link button waits for it. */
	writing: boolean;
	/** The last failure, shown under the header button. */
	error: string | null;
}>({ sprints: null, goalId: null, writing: false, error: null });

/** The goal being linked to, as last loaded; null when not linking. */
export const linkingGoal = (): Goal | null => findGoal(picker.sprints, picker.goalId);

/** Only the latest reload's answer lands: an older one answering late is dropped. */
let latest = 0;

export async function loadSprints(): Promise<void> {
	const seq = ++latest;
	let s: Sprints;
	try {
		s = await unwrap(api.GET('/api/v1/sprints'));
	} catch (e) {
		if (seq === latest) picker.error = errorMessage(e);
		return;
	}
	if (seq !== latest) return;
	picker.sprints = s;
	picker.error = null;
	if (picker.goalId !== null && !findGoal(s, picker.goalId)) picker.goalId = null;
}

export function startLinking(id: number): void {
	picker.goalId = id;
	picker.error = null;
}

export function stopLinking(): void {
	picker.goalId = null;
	picker.error = null;
}

/** Links `key` to the picked goal, or unlinks it when the goal links it directly; then reloads. A 401 from `unwrap`
 *  sends the tab to sign-in, so the promise may never settle, which is fine because a full page load follows. One
 *  write at a time, so a late answer can never undo a newer one. */
export async function toggleLink(key: string): Promise<void> {
	const goal = linkingGoal();
	if (!goal || picker.writing) return;
	picker.writing = true;
	picker.error = null;
	try {
		if (goal.links.includes(key)) await unwrap(api.DELETE('/api/v1/goals/{id}/links/{key}', { params: { path: { id: goal.id, key } } }));
		else await unwrap(api.POST('/api/v1/goals/{id}/links', { params: { path: { id: goal.id } }, body: { items: [key] } }));
		await loadSprints();
	} catch (e) {
		const message = errorMessage(e);
		// The goal may have been removed or its sprint closed meanwhile: the reload ends the mode, and then there is
		// nothing left for the error to belong to.
		await loadSprints();
		if (picker.goalId === goal.id) picker.error = message;
	} finally {
		picker.writing = false;
	}
}

/** Back to the start (tests; nothing in the app needs it). Callers must await any `toggleLink` before resetting. */
export function resetPicker(): void {
	latest++;
	picker.sprints = null;
	picker.goalId = null;
	picker.writing = false;
	picker.error = null;
}
