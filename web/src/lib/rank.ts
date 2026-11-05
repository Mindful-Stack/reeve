// The todo ranking write, shared by the board and the backlog.
import { api, unwrap } from '#lib/api/client.ts';
import type { StoryWithReadiness } from '#lib/api/types.ts';

/** One `POST /stories/{key}/rank` at the moved story's version: the server places it against the complete todo list
 *  (`full`, filters only hide rows) and renumbers the list in one write. Other stories' ranks change without a version
 *  bump (rank is ordering metadata), so an editor open on a neighbour keeps its base. Callers first check
 *  `orderAfterDrop(full, movedKey, before)` and call this only when the placement changes something (so only then do they
 *  clear their notice, go busy and move focus). Refusals, conflicts and other errors are thrown for the caller. */
export async function rankStory(full: StoryWithReadiness[], movedKey: string, before: string | null): Promise<void> {
	const moved = full.find((s) => s.story.key === movedKey);
	if (!moved) return;
	await unwrap(
		api.POST('/api/v1/stories/{key}/rank', {
			params: { path: { key: movedKey } },
			body: { version: moved.story.version, before: before ?? undefined }
		})
	);
}
