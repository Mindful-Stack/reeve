// The delivery move, shared by the board and the story drawer: which step a move needs first, the request itself, and what
// a refusal turns into (an override prompt for a person, a notice otherwise).
import { api, Refused, unwrap } from '#lib/api/client.ts';
import type { Reason, Story } from '#lib/api/types.ts';
import { COLUMN_LABEL, fullLabel, resolutionsFor, type Column, type Resolution } from '#lib/board.ts';
import { isInteractive } from '#lib/me.svelte.ts';

/** What a request to move `s` to `to` needs first: a resolution (a move into done, or a re-resolution within done), the
 *  move itself, or nothing (the story is already there, or a done story has nothing to re-resolve to). */
export function moveStep(s: Pick<Story, 'delivery' | 'resolution'>, to: Column): 'resolve' | 'move' | 'none' {
	if (to === 'done') return resolutionsFor(s.delivery, s.resolution ?? null).length > 0 ? 'resolve' : 'none';
	return to === s.delivery ? 'none' : 'move';
}

export type Refusal = { kind: 'override'; reasons: Reason[] } | { kind: 'notice'; notice: string };

/** A refused move: when the server says an override can get past it (`overridable`), a person gets the override prompt;
 *  anything else (a delivery rule, an automation token) becomes a notice naming every reason. */
export function refusal(key: string, to: Column, reasons: Reason[], overridable: boolean, interactive: boolean): Refusal {
	if (overridable && interactive) return { kind: 'override', reasons };
	return {
		kind: 'notice',
		notice: `${key} was not moved to ${COLUMN_LABEL[to]}: ${reasons.map((r) => fullLabel(r)).join('; ')}.`
	};
}

export type MoveOutcome = { kind: 'moved'; story: Story } | Refusal;

/** `POST /api/v1/stories/{key}/move` with the story's version. A refusal comes back as a `Refusal`; a `Conflict` or any other
 *  `ApiError` is thrown for the caller, which reloads. */
export async function postMove(
	s: Pick<Story, 'key' | 'version'>,
	to: Column,
	opts: { resolution?: Resolution; overrideReason?: string } = {}
): Promise<MoveOutcome> {
	try {
		const story = await unwrap(
			api.POST('/api/v1/stories/{key}/move', {
				params: { path: { key: s.key } },
				body: {
					version: s.version,
					to,
					...(opts.resolution ? { resolution: opts.resolution } : {}),
					...(opts.overrideReason ? { override_reason: opts.overrideReason } : {})
				}
			})
		);
		return { kind: 'moved', story };
	} catch (e) {
		if (e instanceof Refused) return refusal(s.key, to, e.reasons, e.overridable, isInteractive());
		throw e;
	}
}
