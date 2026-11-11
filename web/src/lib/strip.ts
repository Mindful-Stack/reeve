// The detail pages' status strip: what blocks progress, worded for a shared screen.
import { plural } from '#lib/format.ts';

export type BlockerInput = {
	blockingDecisions: { onTitle: string }[];
	storiesNeedingCriteria: number;
	/** Where the stories needing criteria are listed on the page: '#stories' on an epic, '#epics' on an initiative. */
	criteriaAnchor: string;
};
export type Cell =
	| { blocking: true; text: string; detail: string | null; anchor: string }
	| { blocking: false; text: string; detail: null; anchor: null };


export function blockerCell({ blockingDecisions: d, storiesNeedingCriteria: c, criteriaAnchor }: BlockerInput): Cell {
	if (d.length) {
		const places = [...new Set(d.map((x) => x.onTitle))];
		const shown = places.slice(0, 2).join(', ');
		const more = places.length > 2 ? ` and ${places.length - 2} more` : '';
		return { blocking: true, text: plural(d.length, 'blocking decision', 'blocking decisions'), detail: `on ${shown}${more}`, anchor: '#decisions' };
	}
	if (c) return { blocking: true, text: `${plural(c, 'story needs', 'stories need')} criteria`, detail: null, anchor: criteriaAnchor };
	return { blocking: false, text: 'Nothing blocking', detail: null, anchor: null };
}
