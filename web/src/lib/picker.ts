// web/src/lib/picker.ts: the option logic behind lib/ui/Picker.svelte. It narrows a list the caller already has; it
// never fetches, and it never filters stories (kb/ADR-0021 R1 is the server's job).
export type PickerOption = { value: string; label: string };

/** The options whose label holds every whitespace-separated word of `query`, case-insensitively, in list order. */
export function narrow(options: PickerOption[], query: string): PickerOption[] {
	const words = query.toLowerCase().split(/\s+/).filter(Boolean);
	if (words.length === 0) return options;
	return options.filter((o) => {
		const l = o.label.toLowerCase();
		return words.every((w) => l.includes(w));
	});
}

/** `options`, plus the current value when the list does not hold it (a closed epic, a label no longer in use). */
export function withCurrent(options: PickerOption[], value: string): PickerOption[] {
	if (value === '' || options.some((o) => o.value === value)) return options;
	return [...options, { value, label: value }];
}

/** Epics or initiatives as options: "KEY Title", valued by the key. Every site labels them this one way. */
export const keyed = (items: readonly { key: string; title: string }[]): PickerOption[] =>
	items.map((i) => ({ value: i.key, label: `${i.key} ${i.title}` }));
