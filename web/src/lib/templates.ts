// Brief templates in the create forms: when choosing a template may replace what the brief field holds. Pure; the
// picker (#lib/components/TemplatePicker.svelte) holds the state. The forms show a template's text to the author in
// the brief field and send only the brief: they never ask the server to apply a template (`template`,
// `default_template`), so nothing here copies the server's rule (actions::starting_brief).
import type { BriefTemplate } from '#lib/api/types.ts';

/** The brief field's text, and the body the picker last put there ('' before any, and for "No template"). */
export type Filled = { text: string; applied: string };

/** What the picker last put in a draft's brief: the entry (a template's id as text, '' for "No template") and the
 *  body it wrote. It is kept with the draft, so a reopened picker knows its own text from the author's: ownership is
 *  never guessed from the template list, where a text the author typed can equal a template's body. */
export type Applied = { id: string; body: string };
/** A draft the picker has not written to. */
export const NOTHING_APPLIED: Applied = Object.freeze({ id: '', body: '' });

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
