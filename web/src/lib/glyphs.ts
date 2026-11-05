// The glyph beside every key. One key sequence (kb/ADR-0002 R5) means a key alone does not say whether it names
// an initiative, an epic or a story; the set is defined once in the CLI (crates/reeve/src/glyphs.rs) and generated
// here as glyphs.json, which a Rust test keeps in step.
import type { RequiredDecision, Story } from '#lib/api/types.ts';
import set from './glyphs.json';

export type Level = keyof typeof set;

export const levelOfKind = (k: Story['kind']): Level => k;
export const levelOfItem = (t: RequiredDecision['on_type']): Level => t;
export const glyph = (l: Level): string => set[l].glyph;
