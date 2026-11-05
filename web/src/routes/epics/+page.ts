// web/src/routes/epics/+page.ts: the epic index is the backlog's "by epic" view now. Client-side, so it works
// under the static fallback (kb/ADR-0006 R2).
import { redirect } from '@sveltejs/kit';

export const load = () => redirect(307, '/backlog');
