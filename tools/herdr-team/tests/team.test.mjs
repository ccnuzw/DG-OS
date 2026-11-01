import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import os from 'node:os';
import { execFileSync } from 'node:child_process';
import { root, safeRepoPath, checkTask, buildBrief, assertAgent, herdrFailure, withLock, dispatchOnce, createWithIntent, submitOnboarding } from '../../../scripts/herdr-team.mjs';

const baseSha = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim();
const valid = () => ({
  taskId: 'ENG-TEAM-TEST', title: 'Targeted test', risk: 'L1', owner: 'rust', baseSha,
  featureIds: [], acceptanceIds: [], specRefs: [], decisionRefs: [], dependsOn: [], resources: [],
  allowedPaths: ['scripts/herdr-team.mjs'], nonGoals: ['No product behavior changes'],
  verification: [{ command: 'node --test tools/herdr-team/tests/team.test.mjs', level: 'unit', covers: [], notCovered: [] }],
  stopConditions: ['Specification conflict'], timeboxMinutes: 45, maxReviewRounds: 2, humanApproval: 'not-requested',
});

test('valid internal L1 packet requires no new product specification', () => {
  assert.deepEqual(checkTask(valid()), []);
});

test('malformed packets return actionable validation errors', () => {
  for (const value of [null, undefined, [], 'task']) assert.ok(checkTask(value).length > 0);
});

test('template cannot be dispatched without replacing placeholders', () => {
  const template = JSON.parse(fs.readFileSync(path.join(root, 'tools/herdr-team/task.template.json')));
  assert.ok(checkTask(template).some((error) => error.includes('placeholders')));
});

test('absolute, traversal and backslash paths are rejected', () => {
  for (const value of ['/tmp/a', '../a', 'src/../../a', 'src\\a', '', '.']) {
    assert.throws(() => safeRepoPath(value));
  }
});

test('paths stay inside the checkout', () => {
  assert.equal(safeRepoPath('scripts/herdr-team.mjs'), path.join(root, 'scripts/herdr-team.mjs'));
});

test('L2 needs a feature or explicit technical task and spec references', () => {
  const task = { ...valid(), risk: 'L2' };
  assert.ok(checkTask(task).some((e) => e.includes('featureIds')));
  assert.ok(checkTask(task).some((e) => e.includes('specRefs')));
});

test('spec hashes detect changed or fabricated context', () => {
  const task = valid();
  task.specRefs = [{ path: 'AGENTS.md', sha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(root, 'AGENTS.md'))).digest('hex') }];
  assert.deepEqual(checkTask(task), []);
  task.specRefs[0].sha256 = '0'.repeat(64);
  assert.ok(checkTask(task).some((e) => e.includes('Stale')));
});

test('metadata and runtime cannot be granted as source write paths', () => {
  for (const value of ['.git/config', '.herdr-team/state.json', './.git/config', './.herdr-team/state.json']) {
    assert.ok(checkTask({ ...valid(), allowedPaths: [value] }).some((e) => e.includes('cannot grant')));
  }
});

test('missing verification, overlong timeboxes and excessive retry rounds fail', () => {
  const errors = checkTask({ ...valid(), verification: [], timeboxMinutes: 121, maxReviewRounds: 3 });
  assert.ok(errors.some((e) => e.includes('verification')));
  assert.ok(errors.some((e) => e.includes('timebox')));
  assert.ok(errors.some((e) => e.includes('maxReview')));
});

test('bootstrap carries exact authority paths and has no product assignment', () => {
  const brief = buildBrief('rust', { baseSha, roles: { rust: { cwd: '/isolated/rust' } } });
  assert.ok(brief.includes('/isolated/rust'));
  assert.ok(brief.includes(path.join(root, 'AGENTS.md')));
  assert.ok(brief.includes('BOOTSTRAP ONLY'));
  assert.ok(brief.includes('no edits'));
});

test('dispatch requires the configured tool, pane and settled state', () => {
  const role = { kind: 'codex' };
  const slot = { paneId: 'test:p1' };
  for (const state of ['working', 'blocked', 'unknown']) {
    assert.throws(() => assertAgent({ pane_id: 'test:p1', agent: 'codex', agent_status: state }, role, slot));
  }
  assert.throws(() => assertAgent({ pane_id: 'other:p1', agent: 'codex', agent_status: 'idle' }, role, slot));
  assert.throws(() => assertAgent({ pane_id: 'test:p1', agent: 'omp', agent_status: 'idle' }, role, slot));
  assert.doesNotThrow(() => assertAgent({ pane_id: 'test:p1', agent: 'codex', agent_status: 'done' }, role, slot));
});

test('commands are data in task validation and are never executed', () => {
  const task = valid();
  task.verification[0].command = 'this-command-must-never-be-executed';
  assert.deepEqual(checkTask(task), []);
});

test('failed dispatch never prints the complete task in the CLI error', () => {
  const secret = 'sensitive-task-body';
  const error = herdrFailure(['agent', 'prompt', 'dgos-review', secret], { status: 1, message: secret, stderr: secret });
  assert.ok(error.message.includes('agent prompt'));
  assert.ok(error.message.includes('Do not blindly retry'));
  assert.ok(!error.message.includes(secret));
});

function temporary(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'dgos-team-test-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}

test('delivery attempts are persisted before submit and duplicate content is rejected', (t) => {
  const directory = temporary(t);
  let calls = 0;
  dispatchOnce(directory, 'review', 'same content', () => { calls++; return 'ok'; });
  assert.throws(() => dispatchOnce(directory, 'review', 'same content', () => { calls++; }), /already dispatched/);
  assert.equal(calls, 1);
  dispatchOnce(directory, 'review', 'same content', () => { calls++; }, true);
  assert.equal(calls, 2);
});

test('uncertain delivery is not silently retried or written as settled', (t) => {
  const directory = temporary(t);
  assert.throws(() => dispatchOnce(directory, 'rust', 'private task', () => { throw new Error('timeout'); }), /timeout/);
  const ledger = JSON.parse(fs.readFileSync(path.join(directory, 'rust.json')));
  assert.equal(ledger.attempts[0].status, 'uncertain');
  assert.ok(!JSON.stringify(ledger).includes('private task'));
  assert.throws(() => dispatchOnce(directory, 'rust', 'private task', () => {}), /already dispatched/);
});

test('readiness is rechecked under the dispatch lock before recording or submitting', (t) => {
  const directory = temporary(t);
  let submitted = false;
  assert.throws(() => dispatchOnce(directory, 'ui', 'task', () => { submitted = true; }, false,
    () => { throw new Error('agent became busy'); }), /became busy/);
  assert.equal(submitted, false);
  assert.ok(!fs.existsSync(path.join(directory, 'ui.json')));
});

test('failed onboarding readiness does not mark it submitted; API uncertainty does', (t) => {
  const directory = temporary(t);
  const slot = {};
  const submit = () => submitOnboarding(slot, () => {}, () => { throw new Error('API timeout'); });
  assert.throws(() => dispatchOnce(directory, 'spec', 'bootstrap', submit, false,
    () => { throw new Error('busy'); }), /busy/);
  assert.equal(slot.onboardingSubmittedAt, undefined);
  assert.throws(() => dispatchOnce(directory, 'spec', 'bootstrap', submit), /API timeout/);
  assert.ok(slot.onboardingSubmittedAt);
  assert.throws(() => dispatchOnce(directory, 'spec', 'bootstrap', submit), /already dispatched/);
});

test('live and incomplete locks cannot be stolen', (t) => {
  const lock = path.join(temporary(t), 'setup.lock');
  withLock(lock, () => assert.throws(() => withLock(lock, () => {}, () => true), /live or unverifiable/));
  fs.mkdirSync(lock);
  assert.throws(() => withLock(lock, () => {}), /Incomplete lock/);
});

test('only a verifiably dead lock is recovered', (t) => {
  const lock = path.join(temporary(t), 'setup.lock');
  fs.mkdirSync(lock);
  fs.writeFileSync(path.join(lock, 'owner.json'), JSON.stringify({ pid: 12345, token: 'old-owner' }));
  assert.equal(withLock(lock, () => 'recovered', () => false), 'recovered');
  assert.ok(!fs.existsSync(lock));
});

test('creation records intent first and stops retries after an uncertain result', () => {
  const state = {};
  const records = [];
  const persist = (value) => records.push(JSON.parse(JSON.stringify(value)));
  assert.throws(() => createWithIntent(state, { kind: 'workspace', label: 'test' }, () => { throw new Error('interrupted'); }, () => {}, persist));
  assert.equal(records[0].pendingCreation.kind, 'workspace');
  assert.ok(state.pendingCreation.label.startsWith('test ['));
  let called = false;
  assert.throws(() => createWithIntent(state, {}, () => { called = true; }, () => {}, persist), /Unreconciled/);
  assert.equal(called, false);
});

test('successful creation records resource handles before clearing intent', () => {
  const state = {};
  const records = [];
  createWithIntent(state, { kind: 'tab', label: 'checks' }, () => ({ id: 'new-tab' }), (result) => { state.resource = result.id; },
    (value) => records.push(JSON.parse(JSON.stringify(value))));
  assert.ok(records[0].pendingCreation);
  assert.equal(records[1].resource, 'new-tab');
  assert.equal(records[1].pendingCreation, undefined);
});
