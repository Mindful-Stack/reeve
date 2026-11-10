// Typed builders for the wire shapes tests hand to components and helpers. Each returns a complete DTO from the
// generated schema (kb/ADR-0006 R2), so a change to the schema makes the tests that use it fail to type-check instead of
// passing on a shape the server no longer sends. Overrides are a `Partial` of the same DTO: never cast one.
import type {
	ActivityEntry,
	AffectedStory,
	DecisionQueue,
	DecisionQueueEntry,
	Comment,
	Epic,
	Goal,
	Initiative,
	InstanceConfig,
	Me,
	Reason,
	RequiredDecision,
	Sprint,
	SprintView,
	Story,
	StoryWithReadiness
} from '#lib/api/types.ts';

/** A todo story with no epic, owner, criteria or blockers. */
export function story(over: Partial<Story> = {}): Story {
	const key = over.key ?? 'ABC-10';
	return {
		key,
		title: `title of ${key}`,
		kind: 'story',
		epic: null,
		owner: null,
		labels: [],
		rank: 0,
		blocked_by: [],
		prs: [],
		goals: [],
		urgent_mark: null,
		urgent_sources: [],
		criteria: [],
		description: '',
		delivery: 'todo',
		resolution: null,
		external_ref: null,
		version: 1,
		...over
	};
}

/** A `GET /stories` row: ready exactly when `reasons` is empty, as the server reports it. */
export function withReadiness(over: Partial<Story> = {}, reasons: Reason[] = []): StoryWithReadiness {
	return { story: story(over), readiness: { ready: reasons.length === 0, reasons } };
}

export function epic(over: Partial<Epic> = {}): Epic {
	const key = over.key ?? 'ABC-2';
	return {
		key,
		title: `Epic ${key}`,
		brief: '',
		initiative: null,
		discovery_complete: null,
		external_ref: null,
		version: 1,
		urgent_mark: null,
		urgent_sources: [],
		...over
	};
}

export function initiative(over: Partial<Initiative> = {}): Initiative {
	const key = over.key ?? 'ABC-1';
	return {
		key,
		title: `Initiative ${key}`,
		brief: '',
		horizon: 'Now',
		target_month: null,
		owner: null,
		progress: null,
		discovery_complete: null,
		external_ref: null,
		version: 1,
		urgent_mark: null,
		urgent_sources: [],
		...over
	};
}

/** An open, unchecked decision with no record, on a story. */
export function decision(over: Partial<RequiredDecision> = {}): RequiredDecision {
	const id = over.id ?? 1;
	return {
		id,
		on: 'ABC-1',
		on_type: 'story',
		title: `decision ${id}`,
		decider: 'ana@example.com',
		due: null,
		record: null,
		pr_url: null,
		check: { kind: 'unchecked' },
		checked_at: null,
		closed: null,
		created_at: '2026-10-01T00:00:00Z',
		version: 1,
		...over
	};
}

export function activity(over: Partial<ActivityEntry> = {}): ActivityEntry {
	return {
		id: 1,
		at: '2026-11-01T10:00:00Z',
		entity_type: 'story',
		entity_key: 'ABC-1',
		actor: 'erik@example.com',
		action: 'moved',
		via: 'interactive',
		payload: {},
		...over
	};
}

export function comment(over: Partial<Comment> = {}): Comment {
	return {
		id: 1,
		at: '2026-10-01T00:00:00Z',
		author: 'dan@example.com',
		body: '',
		external_ref: null,
		on_decision: null,
		on_epic: null,
		on_initiative: null,
		on_story: null,
		...over
	};
}

/** The signed-in user: an interactive member. */
export function me(over: Partial<Me> = {}): Me {
	return { email: 'me@example.com', role: 'member', token_kind: 'interactive', ...over };
}

export function config(over: Partial<InstanceConfig> = {}): InstanceConfig {
	return {
		horizon_buckets: ['Now', 'Next', 'Later'],
		key_prefix: 'ABC',
		repos: [],
		sign_in_configured: true,
		done_window_days: 14,
		import_enabled: false,
		...over
	};
}

/** A goal of sprint 1 with no links, confidence, suggestion or verdict. */
export function goal(over: Partial<Goal> = {}): Goal {
	const id = over.id ?? 1;
	return {
		id,
		sprint_id: 1,
		outcome: `outcome ${id}`,
		check: '',
		notes: '',
		owner: 'ana@example.com',
		links: [],
		confidence: null,
		confidence_at: null,
		confidence_note: null,
		suggested_confidence: null,
		suggested_by: null,
		suggested_at: null,
		suggested_note: null,
		verdict: null,
		verdict_note: null,
		version: 1,
		...over
	};
}

/** The active sprint 1, with no goals unless given. */
export function sprint(over: Partial<Sprint> = {}): Sprint {
	return {
		id: 1,
		state: 'active',
		starts_on: '2026-11-02',
		ends_on: '2026-11-13',
		opened_at: '2026-11-02T08:00:00Z',
		closed_at: null,
		goals: [],
		warnings: [],
		version: 1,
		...over
	};
}

/** `sprint`'s view with zero counts for each of its goals. */
export function sprintView(s: Sprint = sprint(), over: Partial<SprintView> = {}): SprintView {
	return {
		sprint: s,
		counts: {
			completed: 0,
			upkeep_completed: 0,
			goals: s.goals.map((g) => ({ goal_id: g.id, completed: 0, in_flight: 0, todo: 0, in_review: 0, blocked_decisions: 0 }))
		},
		demo: [],
		upkeep_line: 20,
		...over
	};
}

/** A todo story a decision holds, at the wire's defaults (no owner, not urgent, no window). */
export function blockedStory(over: Partial<AffectedStory> = {}): AffectedStory {
	const key = over.key ?? 'ABC-31';
	return {
		key,
		title: `title of ${key}`,
		kind: 'story',
		delivery: 'todo',
		owner: null,
		urgent: false,
		needed_in: 'beyond',
		...over
	};
}

/** A queue entry in the open group with no stories held and no next step (the server sends `blocking` only with held stories). */
export function queueEntry(over: Partial<DecisionQueueEntry> = {}): DecisionQueueEntry {
	return {
		decision: decision(),
		group: 'open',
		lane: 'open',
		needed_in: 'none',
		needed_in_reason: null,
		next_step: null,
		blocks: [],
		under_way: [],
		...over
	};
}

/** The queue's counts, all zero. */
export function queueSummary(over: Partial<DecisionQueue['summary']> = {}): DecisionQueue['summary'] {
	return {
		open: 0,
		blocking: 0,
		blocked_stories: 0,
		overdue: 0,
		review: 0,
		this_sprint: 0,
		next_sprint: 0,
		...over
	};
}
