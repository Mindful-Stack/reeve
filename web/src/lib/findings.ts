import type { Finding } from '#lib/api/types.ts';

/** One line per advisory finding, worded as the CLI words them (`reeve initiative show`). */
export function findingLabel(f: Finding): string {
	switch (f.finding) {
		case 'empty_brief':
			return 'the brief is empty';
		case 'brief_cites_without_decision':
			return `the brief cites ${f.cites.join(', ')} but no required decision is on this item or above it`;
		case 'discovery_complete_with_open_decision':
			return `discovery is complete while decision #${f.decision_id} "${f.title}" is open`;
		case 'discovery_complete_without_stories':
			return 'discovery is complete and there are no stories under it';
	}
}
