import { describe, expect, it } from 'vitest';
import { followBase, rebase } from './editorBase.ts';

describe('followBase', () => {
	const base = { version: 5, text: 'original' };

	it('advances to the server version when this field is unchanged (an edit elsewhere is no conflict)', () => {
		expect(followBase(base, { version: 6, text: 'original' })).toEqual({ version: 6, text: 'original' });
	});

	it('keeps the starting version when the field changed, so the save conflicts', () => {
		expect(followBase(base, { version: 6, text: 'a colleague wrote this' })).toEqual(base);
	});

	it('advances again once the field is back to the starting text (nothing of a colleague would be lost)', () => {
		const kept = followBase(base, { version: 6, text: 'changed' });
		expect(followBase(kept, { version: 7, text: 'original' })).toEqual({ version: 7, text: 'original' });
	});
});

describe('rebase', () => {
	it('takes the loaded version and text', () => {
		expect(rebase({ version: 9, text: 'now' })).toEqual({ version: 9, text: 'now' });
	});
});
