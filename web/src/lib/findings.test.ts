import { describe, expect, it } from 'vitest';
import { findingLabel } from './findings.ts';

describe('findingLabel', () => {
	it('words every finding as the CLI does', () => {
		expect(findingLabel({ finding: 'empty_brief' })).toBe('the brief is empty');
		expect(findingLabel({ finding: 'brief_cites_without_decision', cites: ['Jira', 'decided'] })).toBe(
			'the brief cites Jira, decided but no required decision is on this item or above it'
		);
		expect(findingLabel({ finding: 'discovery_complete_with_open_decision', decision_id: 7, title: 'Pick a host' })).toBe(
			'discovery is complete while decision #7 "Pick a host" is open'
		);
		expect(findingLabel({ finding: 'discovery_complete_without_stories' })).toBe(
			'discovery is complete and there are no stories under it'
		);
	});
});
