import type { PrStatus, PullRequest } from '#lib/api/types.ts';

export const PR_LABEL = {
	draft: 'draft',
	open: 'open',
	merged: 'merged',
	closed: 'closed'
} satisfies Record<PrStatus, string>;

/** Most advanced first: a merged pull request outranks an open one, then a draft, then a closed one. */
const ORDER: PrStatus[] = ['merged', 'open', 'draft', 'closed'];

/** The one-line summary of a story's pull requests for the card badge. */
export function prSummary(prs: PullRequest[]): { label: string; status: PrStatus | null } {
	if (prs.length === 0) return { label: '', status: null };
	if (prs.length === 1) return { label: PR_LABEL[prs[0].status], status: prs[0].status };
	const status = ORDER.find((o) => prs.some((p) => p.status === o)) ?? null;
	return { label: `${prs.length} pull requests`, status };
}

/** `repo#number · status`, the badge tooltip line for one pull request. */
export function prLine(pr: PullRequest): string {
	return `${pr.repo}#${pr.number} · ${PR_LABEL[pr.status]}`;
}

/** The url only when it is http(s), so a stored `javascript:` or `data:` url never becomes a live link. */
export function safePrUrl(url: string): string | null {
	try {
		const u = new URL(url);
		return u.protocol === 'http:' || u.protocol === 'https:' ? url : null;
	} catch {
		return null;
	}
}
