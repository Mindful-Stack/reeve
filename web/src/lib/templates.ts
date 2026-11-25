// Brief templates in the create forms: when choosing a template may replace what the brief field holds. Pure; the
// picker (#lib/components/TemplatePicker.svelte) holds the state. The forms show a template's text to the author in
// the brief field and send only the brief: they never ask the server to apply a template (`template`,
// `default_template`), so nothing here copies the server's rule (actions::starting_brief). One thing here does rely
// on a rule of the server's: `startedFrom` takes a blank text for no template's, because the server refuses a
// template whose body is blank.
import type { BriefTemplate } from '#lib/api/types.ts';

/** The brief field's text, and the body the picker last put there ('' before any, and for "No template"). */
export type Filled = { text: string; applied: string };

/** Nothing of the author's would be lost by replacing the text: it is blank, or still exactly what the picker put
 *  there. Exact on purpose: one changed character is the author's work. */
export function untouched(f: Filled): boolean {
	return f.text.trim() === '' || f.text === f.applied;
}

export type Pick = { kind: 'fill'; text: string; applied: string } | { kind: 'ask' };

/** Choosing a template with `body` ('' for "No template"): an untouched field takes it; a field the author changed
 *  is left alone, and the picker asks. Typed text is never discarded without that answer. */
export function pick(f: Filled, body: string): Pick {
	return untouched(f) ? { kind: 'fill', text: body, applied: body } : { kind: 'ask' };
}

/** The template the server marked as its level's default, if any. `templates` are one level's. */
export function defaultOf(templates: readonly BriefTemplate[]): BriefTemplate | undefined {
	return templates.find((t) => t.is_default);
}

/** The template a draft's text came from, when the text is still exactly a listed template's body: the default
 *  when it is one of them, else the first listed. For a draft kept from a cancelled dialog: the picker then shows
 *  that entry and treats the text as untouched, instead of taking the template's own text for the author's. A
 *  template's body is never blank (the server refuses one), so a blank text is from none. */
export function startedFrom(templates: readonly BriefTemplate[], text: string): BriefTemplate | undefined {
	const same = templates.filter((t) => t.body === text);
	return same.find((t) => t.is_default) ?? same[0];
}
