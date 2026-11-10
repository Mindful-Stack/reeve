// Unsent decision comments per decision for this session, so a drawer swap, or a move between a row and the drawer,
// keeps the text and its idempotency key (client state that survives navigation lives in a *.svelte.ts module).
import { newDraftKey, type DraftKey } from '#lib/idempotency.ts';

export type CommentDraft = { body: string; id: DraftKey };

export const decisionDrafts: Record<number, CommentDraft> = $state({});

/** The decision's draft, created on first use. Call it from input handlers only: writing `$state` during render throws.
 *  Templates read `decisionDrafts[id]?.body ?? ''`. */
export function draftFor(id: number): CommentDraft {
	return (decisionDrafts[id] ??= { body: '', id: newDraftKey() });
}

/** Forgets every draft; tests call it in `afterEach` (as `resetStreamForTests` does for the event stream). */
export function resetDecisionDraftsForTests(): void {
	for (const k of Object.keys(decisionDrafts)) delete decisionDrafts[Number(k)];
}
