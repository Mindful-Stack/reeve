/** Whether moving focus now would take it from no one: it is inside `form` (the form is about to close, or re-enable its
 *  fields) or on the page itself (a disabled control gave it up). Focus the user moved to another control is theirs. */
export function focusIsFree(form: Element | null | undefined): boolean {
	const a = document.activeElement;
	return !a || a === document.body || !!form?.contains(a);
}
