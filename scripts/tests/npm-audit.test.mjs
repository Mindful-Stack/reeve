// The web audit's gate (scripts/npm-audit.mjs) against hand-written `npm audit --json` reports: `node --test`.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { advisories, gate } from '../npm-audit.mjs';

const advisory = (id, severity) => ({
	source: 1,
	name: 'dep',
	title: `title ${id}`,
	url: `https://github.com/advisories/${id}`,
	severity,
	range: '<1.0.0'
});

// A low advisory in `deep`, reached through `mid` (a string `via`), and a high one listed under two packages.
const report = {
	auditReportVersion: 2,
	vulnerabilities: {
		deep: { severity: 'low', via: [advisory('GHSA-low', 'low')] },
		mid: { severity: 'low', via: ['deep'] },
		a: { severity: 'high', via: [advisory('GHSA-high', 'high')] },
		b: { severity: 'high', via: [advisory('GHSA-high', 'high'), 'a'] }
	}
};

test('one entry per advisory id, with every package that carries it', () => {
	const all = advisories(report);
	assert.deepEqual(
		all.map((a) => [a.id, a.severity, a.packages]),
		[
			['GHSA-low', 'low', ['deep']],
			['GHSA-high', 'high', ['a', 'b']]
		]
	);
});

test('fails on advisories at or above the level, not below', () => {
	assert.deepEqual(gate(report, { level: 'high', ignore: {} }).failing.map((a) => a.id), ['GHSA-high']);
	assert.deepEqual(gate(report, { level: 'critical', ignore: {} }).failing, []);
	assert.deepEqual(
		gate(report, { level: 'low', ignore: {} }).failing.map((a) => a.id),
		['GHSA-low', 'GHSA-high']
	);
});

test('an ignored advisory does not fail the gate', () => {
	const v = gate(report, { level: 'low', ignore: { 'GHSA-high': 'reason' } });
	assert.deepEqual(v.failing.map((a) => a.id), ['GHSA-low']);
	assert.deepEqual(v.stale, []);
});

test('an ignore entry that matches nothing is stale', () => {
	assert.deepEqual(gate(report, { level: 'high', ignore: { 'GHSA-gone': 'fixed upstream' } }).stale, ['GHSA-gone']);
});

test('an ignore entry without a reason and an unknown level are errors', () => {
	assert.throws(() => gate(report, { level: 'high', ignore: { 'GHSA-high': ' ' } }), /no reason/);
	assert.throws(() => gate(report, { level: 'severe', ignore: {} }), /unknown level/);
});

test('a report of another version is refused, not passed as empty', () => {
	assert.throws(() => gate({ advisories: {}, metadata: {} }, { level: 'low', ignore: {} }), /report version/);
});

test('an empty report passes', () => {
	const v = gate({ auditReportVersion: 2, vulnerabilities: {} }, { level: 'low', ignore: {} });
	assert.deepEqual([v.all, v.failing, v.stale], [[], [], []]);
});
