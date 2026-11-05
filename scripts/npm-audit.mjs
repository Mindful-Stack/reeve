#!/usr/bin/env node
// The web client's dependency audit (`mise run audit-web`): `npm audit --json` for web/, gated by severity, with known
// advisories ignored only through scripts/npm-audit-ignore.json, where every entry carries its reason. npm audit has no
// ignore list of its own; this is the one place one lives.
//
//   node scripts/npm-audit.mjs --level <low|moderate|high|critical>
//
// The whole tree is audited, dev dependencies included: npm's dev/production split does not say what ships (svelte and
// @sveltejs/kit are devDependencies whose runtime is bundled into the client). Fails when an advisory at or above
// --level is not ignored, and on an ignore entry no advisory matches any more, so a fixed advisory's ignore is removed
// instead of lingering.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const SEVERITIES = ['info', 'low', 'moderate', 'high', 'critical'];

/** Every advisory in an `npm audit --json` (report version 2) document, one entry per advisory id. */
export function advisories(report) {
	const found = new Map();
	for (const [pkg, v] of Object.entries(report.vulnerabilities ?? {})) {
		for (const via of v.via ?? []) {
			// A string `via` names another vulnerable package (a transitive path); the advisory is listed under that one.
			if (typeof via !== 'object' || !via.url) continue;
			const id = via.url.split('/').pop();
			const a = found.get(id) ?? { id, title: via.title, severity: via.severity, url: via.url, packages: [] };
			if (!a.packages.includes(pkg)) a.packages.push(pkg);
			found.set(id, a);
		}
	}
	return [...found.values()];
}

/** The verdict for one report: the advisories that fail the gate and the ignore entries matching nothing. */
export function gate(report, { level, ignore }) {
	const min = SEVERITIES.indexOf(level);
	if (min < 0) throw new Error(`unknown level ${level}; one of ${SEVERITIES.join(', ')}`);
	for (const [id, reason] of Object.entries(ignore)) {
		if (typeof reason !== 'string' || reason.trim() === '') throw new Error(`ignore entry ${id} has no reason`);
	}
	// Any other shape (npm 6's v1 report, a future format) would list no advisories and pass: refuse it instead.
	if (report.auditReportVersion !== 2) throw new Error(`unexpected npm audit report version ${report.auditReportVersion}`);
	const all = advisories(report);
	const failing = all.filter((a) => SEVERITIES.indexOf(a.severity) >= min && !(a.id in ignore));
	const seen = new Set(all.map((a) => a.id));
	const stale = Object.keys(ignore).filter((id) => !seen.has(id));
	return { all, failing, stale };
}

function main(argv) {
	const levelAt = argv.indexOf('--level');
	const level = levelAt >= 0 ? argv[levelAt + 1] : undefined;
	if (!level) throw new Error('usage: npm-audit.mjs --level <low|moderate|high|critical>');
	const root = dirname(dirname(fileURLToPath(import.meta.url)));
	const ignore = JSON.parse(readFileSync(join(root, 'scripts', 'npm-audit-ignore.json'), 'utf8'));
	delete ignore.$comment;
	let out;
	try {
		// --include=dev overrides an inherited omit (NODE_ENV=production, npm_config_omit=dev): svelte and kit are
		// devDependencies whose runtime ships in the bundle, so the whole tree is audited wherever this runs.
		out = execFileSync('npm', ['audit', '--json', '--include=dev'], {
			cwd: join(root, 'web'),
			encoding: 'utf8',
			maxBuffer: 64 * 1024 * 1024
		});
	} catch (e) {
		// npm audit exits non-zero whenever it finds anything; the JSON on stdout is still the report.
		if (!e.stdout) throw e;
		out = e.stdout;
	}
	const report = JSON.parse(out);
	if (report.error) throw new Error(`npm audit failed: ${JSON.stringify(report.error)}`);
	const { all, failing, stale } = gate(report, { level, ignore });
	for (const a of all) {
		const mark = failing.includes(a) ? 'FAIL' : a.id in ignore ? 'ignored' : 'below level';
		console.log(`${mark}: ${a.severity} ${a.id} ${a.title} (${a.packages.join(', ')}) ${a.url}`);
		if (a.id in ignore) console.log(`  reason: ${ignore[a.id]}`);
	}
	for (const id of stale) console.log(`FAIL: ignore entry ${id} matches no advisory any more; remove it`);
	const ok = failing.length === 0 && stale.length === 0;
	console.log(`npm audit (all dependencies, failing at ${level} and above): ${all.length} advisories, ${ok ? 'ok' : 'FAILED'}`);
	return ok ? 0 : 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
	process.exitCode = main(process.argv.slice(2));
}
