#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const config = JSON.parse(fs.readFileSync(path.join(root, 'tools/herdr-team/team.json'), 'utf8'));
const runtime = path.join(root, '.herdr-team');
const statePath = path.join(runtime, 'state.json');
const read = (file) => fs.readFileSync(file, 'utf8');
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
const digest = (file) => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const roleById = (id) => {
  const role = config.roles.find((r) => r.id === id);
  if (!role) throw new Error(`Unknown role: ${id}`);
  return role;
};

export function herdrFailure(args, error) {
  // execFileSync's default message includes argv, including the entire task prompt.
  return new Error(`Herdr ${args.slice(0, 2).join(' ')} failed (${error.status ?? error.code ?? 'invalid response'}). Inspect the target UI; a submitted prompt may still be running. Do not blindly retry.`);
}

function herdr(...args) {
  if (process.env.HERDR_ENV !== '1') throw new Error('Run this command inside a Herdr-managed pane (HERDR_ENV=1).');
  try {
    return JSON.parse(execFileSync('herdr', args, { cwd: root, encoding: 'utf8', timeout: 150_000, stdio: ['ignore', 'pipe', 'pipe'] })).result;
  } catch (error) { throw herdrFailure(args, error); }
}

function loadState() {
  const state = JSON.parse(read(statePath));
  if (state.root !== root) throw new Error('Registry belongs to another checkout; inspect it manually.');
  return state;
}

function saveState(state) {
  fs.mkdirSync(runtime, { recursive: true, mode: 0o700 });
  const temp = `${statePath}.tmp`;
  fs.writeFileSync(temp, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  fs.renameSync(temp, statePath);
}

export function withLock(directory, action, isAlive = (pid) => {
  try { process.kill(pid, 0); return true; } catch (error) { return error.code !== 'ESRCH'; }
}) {
  fs.mkdirSync(path.dirname(directory), { recursive: true });
  const ownerFile = path.join(directory, 'owner.json');
  const owner = { pid: process.pid, token: crypto.randomUUID() };
  try { fs.mkdirSync(directory); }
  catch (error) {
    if (error.code !== 'EEXIST') throw error;
    let previous;
    try { previous = JSON.parse(read(ownerFile)); }
    catch { throw new Error('Incomplete lock; inspect it manually, do not steal it.'); }
    if (!Number.isInteger(previous.pid) || previous.pid <= 0 || typeof previous.token !== 'string' || isAlive(previous.pid)) {
      throw new Error('Lock owner is live or unverifiable; not taking over.');
    }
    // Quarantine only a verifiably dead owner's lock; never kill its process.
    const stale = `${directory}.stale-${crypto.randomUUID()}`;
    fs.renameSync(directory, stale);
    fs.unlinkSync(path.join(stale, 'owner.json'));
    fs.rmdirSync(stale);
    fs.mkdirSync(directory);
  }
  fs.writeFileSync(ownerFile, JSON.stringify(owner), { mode: 0o600 });
  try { return action(); }
  finally {
    const current = JSON.parse(read(ownerFile));
    if (current.token !== owner.token) throw new Error('Lock ownership changed; refusing to remove it.');
    fs.unlinkSync(ownerFile);
    fs.rmdirSync(directory);
  }
}

export function dispatchOnce(directory, role, message, submit, resendConfirmed = false, readyCheck = () => {}) {
  const hash = crypto.createHash('sha256').update(message).digest('hex');
  return withLock(path.join(directory, `${role}.lock`), () => {
    readyCheck();
    const ledgerPath = path.join(directory, `${role}.json`);
    const ledger = fs.existsSync(ledgerPath) ? JSON.parse(read(ledgerPath)) : { attempts: [] };
    if (!resendConfirmed && ledger.attempts.some((entry) => entry.sha256 === hash)) {
      throw new Error('This content was already dispatched or attempted. Read the target UI; only use --resend-confirmed after explicitly confirming a resend is necessary.');
    }
    const attempt = { id: crypto.randomUUID(), sha256: hash, attemptedAt: new Date().toISOString(), status: 'uncertain' };
    ledger.attempts.push(attempt);
    const persist = () => {
      const temp = `${ledgerPath}.${process.pid}.tmp`;
      fs.writeFileSync(temp, `${JSON.stringify(ledger, null, 2)}\n`, { mode: 0o600 });
      fs.renameSync(temp, ledgerPath);
    };
    // A timeout is not proof of non-delivery; save before crossing the API boundary.
    persist();
    const result = submit();
    attempt.status = 'settled';
    persist();
    return result;
  });
}

export function createWithIntent(state, intent, create, register, persist = saveState) {
  if (state.pendingCreation) throw new Error('Unreconciled creation intent; run doctor and inspect resources before continuing.');
  const pending = { ...intent, label: `${intent.label} [${crypto.randomUUID()}]`, recordedAt: new Date().toISOString() };
  state.pendingCreation = pending;
  persist(state);
  const created = create(pending.label);
  register(created);
  delete state.pendingCreation;
  persist(state);
  return created;
}

export function submitOnboarding(slot, persist, submit) {
  // Called only after the locked readiness check and durable dispatch intent.
  slot.onboardingSubmittedAt = new Date().toISOString();
  persist();
  return submit();
}

export function safeRepoPath(value) {
  if (typeof value !== 'string' || !value || path.isAbsolute(value) || value.includes('\\') || value.split('/').includes('..')) {
    throw new Error(`Expected a repository-relative path: ${value}`);
  }
  const resolved = path.resolve(root, value);
  if (resolved === root) throw new Error('The repository root is not an allowed write scope.');
  if (!resolved.startsWith(`${root}${path.sep}`)) throw new Error('Path escapes the repository.');
  // Check existing ancestors too: a non-existing child of a symlink can still escape.
  let existing = resolved;
  while (!fs.existsSync(existing)) existing = path.dirname(existing);
  const real = fs.realpathSync(existing);
  if (real !== fs.realpathSync(root) && !real.startsWith(`${fs.realpathSync(root)}${path.sep}`)) {
    throw new Error('Path follows a symlink outside the repository.');
  }
  return resolved;
}

export function checkTask(task) {
  if (!task || typeof task !== 'object' || Array.isArray(task)) return ['Task packet must be an object'];
  const errors = [];
  const need = (condition, message) => { if (!condition) errors.push(message); };
  need(/^[a-zA-Z0-9][a-zA-Z0-9_-]{1,79}$/.test(task.taskId ?? ''), 'taskId must be a stable identifier');
  need(typeof task.title === 'string' && task.title.trim().length > 0, 'title is required');
  need(['L0', 'L1', 'L2', 'L3'].includes(task.risk), 'risk must be L0-L3');
  need(config.roles.some((r) => r.id === task.owner), 'owner must name a configured role');
  need(/^[a-f0-9]{40}$/.test(task.baseSha ?? ''), 'baseSha must be a full commit SHA');
  if (/^[a-f0-9]{40}$/.test(task.baseSha ?? '')) {
    try { git('cat-file', '-e', `${task.baseSha}^{commit}`); } catch { errors.push('baseSha is not a local commit'); }
  }
  for (const field of ['featureIds', 'acceptanceIds', 'specRefs', 'decisionRefs', 'dependsOn', 'resources']) {
    need(Array.isArray(task[field]), `${field} must be an array`);
  }
  for (const field of ['nonGoals', 'stopConditions', 'verification', 'allowedPaths']) {
    need(Array.isArray(task[field]) && task[field].length > 0, `${field} must be a non-empty array`);
  }
  need(Number.isInteger(task.timeboxMinutes) && task.timeboxMinutes > 0 && task.timeboxMinutes <= 120, 'timeboxMinutes must be 1-120');
  need(Number.isInteger(task.maxReviewRounds) && task.maxReviewRounds >= 0 && task.maxReviewRounds <= config.maxReviewRounds, 'maxReviewRounds must be 0-2');
  need(typeof task.humanApproval === 'string' && task.humanApproval.length > 0, 'humanApproval must describe approval scope or not-requested');
  if (['L2', 'L3'].includes(task.risk)) {
    need(task.featureIds?.length > 0 || Boolean(task.technicalTask), 'L2/L3 requires featureIds or an explicit technicalTask');
    need(task.specRefs?.length > 0, 'L2/L3 requires authoritative specRefs with hashes');
  }
  for (const value of Array.isArray(task.allowedPaths) ? task.allowedPaths : []) {
    try {
      const normalized = path.relative(root, safeRepoPath(value));
      need(!['.git', '.herdr-team'].some((name) => normalized === name || normalized.startsWith(`${name}${path.sep}`)), 'allowedPaths cannot grant repository metadata or team runtime');
    } catch (error) { errors.push(error.message); }
  }
  for (const ref of Array.isArray(task.specRefs) ? task.specRefs : []) {
    try {
      const file = safeRepoPath(ref.path);
      need(/^[a-f0-9]{64}$/.test(ref.sha256 ?? ''), 'specRefs require sha256');
      need(digest(file) === ref.sha256, `Stale spec reference: ${ref.path}`);
    } catch (error) { errors.push(`Invalid spec reference: ${error.message}`); }
  }
  for (const test of Array.isArray(task.verification) ? task.verification : []) {
    need(typeof test.command === 'string' && test.command.trim().length > 0, 'verification.command is required');
    need(['static', 'unit', 'mock', 'compatibility-smoke', 'external', 'webview', 'desktop-e2e', 'release'].includes(test.level), 'verification.level is invalid');
    need(Array.isArray(test.covers) && Array.isArray(test.notCovered), 'verification requires covers and notCovered arrays');
  }
  if (JSON.stringify(task).includes('REPLACE-WITH')) errors.push('Replace all template placeholders before dispatch');
  return errors;
}

export function buildBrief(id, state) {
  const role = roleById(id);
  const slot = state.roles[id];
  if (!slot) throw new Error(`Role not set up: ${id}`);
  return [
    'You are a member of the user-authorized DG-OS Herdr team. Respond in Chinese.',
    `Role: ${role.name}. Your worktree: ${slot.cwd}. Initial baseline: ${state.baseSha}.`,
    `Control checkout (read only unless you are lead): ${root}.`,
    `First read ${path.join(root, 'AGENTS.md')} and ${path.join(root, config.workflow)}.`,
    `Then read your role instructions: ${path.join(root, role.prompt)}.`,
    'These setup files may not yet exist in your baseline worktree; read the exact absolute paths above.',
    'Follow repository authority order and spec-docs-pro for relevant documentation tasks.',
    'BOOTSTRAP ONLY: read the small onboarding scope in your role file; no edits, commits, business tasks, full audit, test suites, agent spawning, or paid external tools.',
    'Do not inspect credential files or other projects. Do not answer permission/trust/auth dialogs for anyone else.',
    'After onboarding, reply with your role, worktree and baseline, authority/stop rules, verification boundaries, and READY or a concrete blocker. Then stop and wait for a user/lead task packet.',
    `Lead agent name: dgos-lead. Team CLI: node ${path.join(root, 'scripts/herdr-team.mjs')}.`,
    'No background polling or automatic next task. Herdr done is not product acceptance.',
  ].join('\n');
}

function slotFor(state, id) {
  roleById(id);
  const slot = state.roles[id];
  if (!slot) throw new Error(`Run setup first: ${id}`);
  return slot;
}

export function assertAgent(agent, role, slot) {
  if (agent.pane_id !== slot.paneId || agent.agent !== role.kind) throw new Error('Agent identity/pane mismatch; inspect before sending.');
  if (!['idle', 'done'].includes(agent.agent_status)) throw new Error(`Agent is ${agent.agent_status}; inspect its UI, do not resubmit blindly.`);
}

function agentInfo(role, slot) {
  const result = herdr('agent', 'get', role.name);
  const agent = result.agent;
  if (!agent || typeof agent !== 'object' || agent.pane_id !== slot.paneId) {
    throw new Error('Named agent response does not match the registered pane.');
  }
  return agent;
}

function setup() {
  if (process.env.HERDR_ENV !== '1') throw new Error('Run setup inside Herdr.');
  fs.mkdirSync(runtime, { recursive: true, mode: 0o700 });
  return withLock(path.join(runtime, 'control.lock'), () => {
    const state = fs.existsSync(statePath) ? loadState() : { schemaVersion: 1, root, baseSha: git('rev-parse', 'HEAD'), roles: {} };
    if (state.pendingCreation) throw new Error('Unreconciled creation intent; run doctor. Do not retry creation or discard the intent without inspecting the recorded resource.');
    if (!state.roles.lead) {
      createWithIntent(state, { kind: 'workspace', role: 'lead', cwd: root, label: roleById('lead').label },
        (label) => herdr('workspace', 'create', '--cwd', root, '--label', label, '--no-focus'),
        (created) => { state.roles.lead = { workspaceId: created.workspace.workspace_id, tabId: created.tab.tab_id, paneId: created.root_pane.pane_id, cwd: root }; });
    }
    for (const role of config.roles) {
      if (!state.roles[role.id]) {
        const cwd = path.join(runtime, 'worktrees', role.id);
        const branch = `team/dgos/${role.id}`;
        createWithIntent(state, { kind: 'worktree', role: role.id, cwd, branch, label: role.label },
          (label) => herdr('worktree', 'create', '--workspace', state.roles.lead.workspaceId, '--branch', branch, '--base', state.baseSha, '--path', cwd, '--label', label, '--no-focus'),
          (created) => { state.roles[role.id] = { workspaceId: created.workspace.workspace_id, tabId: created.tab.tab_id, paneId: created.root_pane.pane_id, cwd, branch }; });
      }
      const slot = slotFor(state, role.id);
      herdr('pane', 'get', slot.paneId);
      herdr('workspace', 'rename', slot.workspaceId, role.label);
      herdr('tab', 'rename', slot.tabId, role.label.split('|')[1].trim());
      herdr('pane', 'rename', slot.paneId, role.name);
      if (!slot.utilityPaneId) {
        createWithIntent(state, { kind: 'tab', role: role.id, workspaceId: slot.workspaceId, cwd: slot.cwd, label: `02 ${role.utilityLabel}` },
          (label) => herdr('tab', 'create', '--workspace', slot.workspaceId, '--cwd', slot.cwd, '--label', label, '--no-focus'),
          (created) => { slot.utilityPaneId = created.root_pane.pane_id; slot.utilityTabId = created.tab.tab_id; });
      } else {
        const utility = herdr('pane', 'get', slot.utilityPaneId).pane;
        if (utility.tab_id === slot.tabId) {
          const moved = herdr('pane', 'move', slot.utilityPaneId, '--new-tab', '--workspace', slot.workspaceId, '--label', `02 ${role.utilityLabel}`, '--no-focus').move_result;
          slot.utilityPaneId = moved.pane.pane_id;
          slot.utilityTabId = moved.pane.tab_id;
        } else {
          slot.utilityTabId = utility.tab_id;
        }
        saveState(state);
      }
      herdr('tab', 'rename', slot.utilityTabId, `02 ${role.utilityLabel}`);
      herdr('pane', 'rename', slot.utilityPaneId, role.utilityLabel);
    }
    saveState(state);
    console.log(JSON.stringify(state, null, 2));
  });
}

function selectedRoles(value) {
  if (value === 'all') return config.roles;
  return [roleById(value)];
}

function start(value) {
  const state = loadState();
  for (const role of selectedRoles(value)) {
    const slot = slotFor(state, role.id);
    const pane = herdr('pane', 'get', slot.paneId).pane;
    if (pane.agent) {
      const agent = agentInfo(role, slot);
      console.log(`${role.id}: existing ${agent.agent_status}, not restarting`);
      continue;
    }
    // Herdr additionally validates that the terminal has an available foreground shell.
    const result = herdr('agent', 'start', role.name, '--kind', role.kind, '--pane', slot.paneId, '--timeout', '60000', '--', ...role.args);
    console.log(JSON.stringify(result));
  }
}

function onboard(value) {
  return withLock(path.join(runtime, 'control.lock'), () => {
  const state = loadState();
  for (const role of selectedRoles(value)) {
    const slot = slotFor(state, role.id);
    if (slot.onboardingSubmittedAt) {
      console.log(`${role.id}: onboarding already attempted; read agent before any intentional resend`);
      continue;
    }
    assertAgent(agentInfo(role, slot), role, slot);
    const brief = buildBrief(role.id, state);
    const result = dispatchOnce(path.join(runtime, 'dispatches'), role.id, brief,
      () => submitOnboarding(slot, () => saveState(state),
        () => herdr('agent', 'prompt', role.name, brief, '--wait', '--timeout', '120000')), false,
      () => assertAgent(agentInfo(role, slot), role, slot));
    console.log(JSON.stringify(result));
  }
  });
}

function status() {
  const state = loadState();
  const agents = herdr('agent', 'list').agents;
  console.log(`Baseline ${state.baseSha}; control ${root}`);
  for (const role of config.roles) {
    const slot = slotFor(state, role.id);
    const agent = agents.find((a) => a.pane_id === slot.paneId);
    console.log(`${role.name.padEnd(12)} ${role.kind.padEnd(9)} ${(agent?.agent_status ?? 'shell/offline').padEnd(14)} ${slot.workspaceId} ${slot.paneId} ${slot.cwd}`);
  }
}

function doctor() {
  const workspaces = herdr('workspace', 'list').workspaces;
  console.log(execFileSync('herdr', ['status'], { encoding: 'utf8' }));
  console.log(execFileSync('herdr', ['integration', 'status'], { encoding: 'utf8' }));
  const state = loadState();
  if (state.pendingCreation) {
    const pending = state.pendingCreation;
    const resources = pending.kind === 'tab'
      ? herdr('tab', 'list', '--workspace', pending.workspaceId).tabs : workspaces;
    console.log(JSON.stringify({ pendingCreation: pending, matchingResources: resources.filter((item) => item.label === pending.label) }, null, 2));
    throw new Error('Reconcile this intent against the live resource, branch and path before editing the local registry. No new resource has been created by doctor.');
  }
  console.log(`Control HEAD ${git('rev-parse', 'HEAD')}; initial baseline ${state.baseSha}`);
  for (const role of config.roles) {
    const slot = slotFor(state, role.id);
    const pane = herdr('pane', 'get', slot.paneId).pane;
    if (fs.realpathSync(pane.cwd) !== fs.realpathSync(slot.cwd)) throw new Error(`Wrong pane cwd: ${role.id}`);
    if (role.id !== 'lead') {
      const branch = execFileSync('git', ['branch', '--show-current'], { cwd: slot.cwd, encoding: 'utf8' }).trim();
      if (branch !== slot.branch) throw new Error(`Unexpected branch for ${role.id}: ${branch}`);
    }
    console.log(`${role.id}: registered pane and checkout present`);
  }
}

export function main(args) {
  const [command, value, file, option] = args;
  if (command === 'setup') return setup();
  if (command === 'start') return start(value);
  if (command === 'onboard') return onboard(value);
  if (command === 'status') return status();
  if (command === 'doctor') return doctor();
  if (command === 'brief') return console.log(buildBrief(value, loadState()));
  if (command === 'check-task') {
    const errors = checkTask(JSON.parse(read(safeRepoPath(value))));
    if (errors.length) throw new Error(errors.join('\n'));
    return console.log('Task packet structure, local base and spec hashes are valid; approval/dependencies/gates still require review.');
  }
  if (command === 'send') {
    const role = roleById(value);
    const slot = slotFor(loadState(), value);
    const message = read(safeRepoPath(file));
    if (!message.trim()) throw new Error('Cannot send an empty task.');
    if (option && option !== '--resend-confirmed') throw new Error('Unknown send option.');
    assertAgent(agentInfo(role, slot), role, slot);
    return console.log(JSON.stringify(dispatchOnce(path.join(runtime, 'dispatches'), role.id, message,
      () => herdr('agent', 'prompt', role.name, message, '--wait', '--timeout', '120000'), option === '--resend-confirmed',
      () => assertAgent(agentInfo(role, slot), role, slot)), null, 2));
  }
  throw new Error('Usage: herdr-team.mjs setup|start ROLE|onboard ROLE|status|doctor|brief ROLE|check-task FILE|send ROLE FILE');
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(process.argv.slice(2)); }
  catch (error) { console.error(`herdr-team: ${error.message}`); process.exitCode = 1; }
}
