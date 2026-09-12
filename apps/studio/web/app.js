'use strict';
const $ = id => document.getElementById(id);
let runs = [], selected = null, snapshot = null, pollTask = null, refreshQueued = false, signature = '', toastTimer;
const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const pretty = value => JSON.stringify(value, null, 2);
function stateName(run) {
  if (run?.active) return 'Running';
  if (run?.state?.outcome?.Completed !== undefined) return 'Completed';
  if (run?.state?.outcome?.Failed !== undefined) return 'Failed';
  return run?.state ? 'Interrupted' : 'Starting';
}
function title(run) { const text = run?.state?.objective?.text || 'Preparing run…'; return text.match(/^(?:create|write) file (.*?) (?:with content|::)/s)?.[1] || text; }
function toast(message) { $('toast').textContent = message; $('toast').hidden = false; clearTimeout(toastTimer); toastTimer = setTimeout(() => $('toast').hidden = true, 4500); }
async function api(path, body) {
  const response = await fetch(path, body === undefined ? {cache:'no-store'} : {method:'POST', headers:{'Content-Type':'application/json','X-Klyne-Request':'1'}, body:JSON.stringify(body)});
  const result = await response.json(); if (!response.ok) throw new Error(result.error || 'Request failed'); return result;
}
function renderRuns() {
  const query = $('run-search').value.toLowerCase();
  const filtered = runs.filter(r => `${title(r)} ${r.id} ${stateName(r)}`.toLowerCase().includes(query));
  $('run-count').textContent = String(runs.length);
  const markup = filtered.length
    ? filtered.map(run => `<button type="button" class="run-item" aria-pressed="${run.id === selected}" data-run="${escape(run.id)}"><span class="file-icon" aria-hidden="true">▤</span><span class="run-item-copy"><span class="run-item-title">${escape(title(run))}</span><span class="run-item-state">${stateName(run)}</span></span><span class="run-arrow" aria-hidden="true">↗</span></button>`).join('')
    : (runs.length ? '<p>No matching runs.</p>' : '<p>No runs yet.</p>');
  if ($('run-list').innerHTML !== markup) {
    const focused = document.activeElement?.closest('[data-run]')?.dataset.run;
    $('run-list').innerHTML = markup;
    if (focused) [...$('run-list').querySelectorAll('[data-run]')].find(button => button.dataset.run === focused)?.focus({preventScroll:true});
  }
}
async function selectRun(id) {
  if (id === selected) return;
  selected = id; signature = ''; snapshot = null;
  $('run-content').hidden = true; $('empty-state').hidden = true;
  $('result-loading').hidden = false;
  $('result-loading').textContent = 'Opening your run…';
  $('event-search').value = '';
  renderRuns(); await refresh();
}
function renderSnapshot() {
  $('empty-state').hidden = !!selected; $('run-content').hidden = !selected;
  if (!snapshot) return;
  $('result-loading').hidden = true;
  const state = snapshot.state || {}, status = stateName(snapshot), budget = state.tool_budget || {};
  $('run-id').textContent = snapshot.id; $('run-title').textContent = title(snapshot);
  const provider = snapshot.provider || {kind:'demo'};
  $('run-provider').textContent = `${providerNames[provider.kind] || provider.kind}${provider.model ? ' / ' + provider.model : ''}`;
  $('status-badge').textContent = status;
  $('status-badge').dataset.state = status;
  $('status-description').textContent = ({Completed:'Created, checked, and saved. A small idea made real.', Running:'Your file is taking shape. Progress updates automatically.', Starting:'Getting everything ready for this run.', Failed:'This run needs attention. See the details below.', Interrupted:'Progress is saved. Review the run before continuing.'})[status];
  $('budget-progress').max = Math.max(1, budget.limit ?? 32);
  $('budget-progress').value = budget.used ?? 0;
  $('tool-used').textContent = budget.used ?? 0; $('tool-limit').textContent = ` / ${budget.limit ?? '—'}`;
  $('stop').hidden = !snapshot.active;
  $('resume').hidden = !!snapshot.active || !snapshot.state || !!state.outcome || !!state.pending;
  const error = snapshot.error || state.outcome?.Failed || (state.pending ? 'An interrupted action needs reconciliation. This workspace will not replay uncertain effects.' : '');
  $('run-error').hidden = !error; $('run-error').textContent = error;
  const query = $('event-search').value.toLowerCase();
  const events = (snapshot.events || []).filter(e => `${e.kind} ${e.detail} ${e.seq}`.toLowerCase().includes(query));
  $('event-ledger').innerHTML = events.length
    ? events.map(e => `<details class="event-row"><summary>#${String(e.seq).padStart(3,'0')} ${escape(e.kind)}</summary><pre>${escape(e.detail)}</pre></details>`).join('')
    : '<p>No matching events.</p>';
  $('evidence-list').innerHTML = (state.history || []).map(([action, observation], i) => {
    const kind = Object.keys(action || {})[0] || 'Unknown';
    return `<details class="evidence-item"><summary><span>${i + 1}. ${escape(kind)}</span><span class="observation-state">${observation.ok ? '✓ Observed' : 'Failed'}</span></summary><pre>${escape(pretty({action, observation}))}</pre></details>`;
  }).join('') || '<p>Tool observations will appear here after execution.</p>';
}
function refresh() {
  if (pollTask) { refreshQueued = true; return pollTask; }
  pollTask = (async () => {
    do { refreshQueued = false; await fetchSnapshot(); } while (refreshQueued);
  })().finally(() => { pollTask = null; });
  return pollTask;
}
async function fetchSnapshot() {
  try {
    const data = await api('/api/runs'); runs = data.runs;
    if (!selected && runs.length) selected = runs[0].id;
    renderRuns();
    if (selected) {
      const id = selected, fresh = await api(`/api/runs/${selected}`);
      if (id === selected) { snapshot = fresh; const next = pretty(fresh); if (next !== signature) { signature = next; renderSnapshot(); } }
      else { refreshQueued = true; }
    } else renderSnapshot();
    $('connection').textContent = 'Connected'; $('connection').dataset.state = 'connected';
  } catch (e) {
    $('connection').textContent = 'Offline — retrying…'; $('connection').dataset.state = 'offline';
    if (!$('result-loading').hidden) $('result-loading').textContent = 'Could not open this run. Retrying automatically…';
  }
}
document.addEventListener('click', e => { const run = e.target.closest('[data-run]'); if (run) selectRun(run.dataset.run); });
$('new-run').onclick = () => { $('file-path').focus(); window.scrollTo({top: 0}); };
$('run-search').oninput = renderRuns;
$('event-search').oninput = renderSnapshot;
$('refresh').onclick = async () => { signature = ''; await refresh(); toast($('connection').dataset.state === 'connected' ? 'Workspace refreshed.' : 'Could not connect. Retrying automatically.'); };
const starters = {
  note: {path:'notes/my-note.txt', contents:'A thought worth keeping.\n\n'},
  checklist: {path:'notes/checklist.txt', contents:'Today\n\n[ ] Pick one thing to focus on\n[ ] Take the first small step\n[ ] Celebrate the progress\n'}
};
document.querySelectorAll('[data-starter]').forEach(button => {
  button.onclick = () => {
    const starter = starters[button.dataset.starter];
    $('file-path').value = starter.path; $('file-contents').value = starter.contents;
    $('form-error').textContent = ''; $('file-contents').focus();
    toast('Example added. Make it yours before creating the file.');
  };
});
$('budget').addEventListener('invalid', () => { $('budget').closest('details').open = true; });
$('run-form').onsubmit = async e => {
  e.preventDefault();
  const path = $('file-path').value.trim(), contents = $('file-contents').value;
  if (!path || /\s|\\|:/.test(path) || path.split('/').some(s => !s || s === '.' || s === '..')) { $('form-error').textContent = 'Use a relative path such as notes/hello.txt, without spaces or traversal.'; return; }
  $('launch-button').disabled = true; $('form-error').textContent = '';
  const launchLabel = $('launch-button').innerHTML;
  $('launch-button').textContent = 'Creating…'; $('run-form').setAttribute('aria-busy', 'true');
  try {
    const result = await api('/api/runs', {objective: `create file ${path} with content ${contents}`, budget: Number($('budget').value), provider: providerConfig()});
    selected = result.id; signature = ''; await refresh(); toast('Run launched.');
  } catch (error) { $('form-error').textContent = error.message; }
  finally { $('launch-button').disabled = false; $('launch-button').innerHTML = launchLabel; $('run-form').removeAttribute('aria-busy'); }
};
for (const action of ['stop', 'resume']) $(action).onclick = async () => {
  const button = $(action); button.disabled = true;
  try { await api(`/api/runs/${selected}/${action}`, {}); signature = ''; await refresh(); toast(action === 'stop' ? 'Stop requested.' : 'Run resumed.'); }
  catch (e) { toast(e.message); }
  finally { button.disabled = false; }
};
$('export').onclick = () => {
  if (!snapshot) return;
  const blob = new Blob([pretty(snapshot)], {type: 'application/json'}), url = URL.createObjectURL(blob), a = document.createElement('a');
  a.href = url; a.download = `klyne-${selected}-evidence.json`; a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000); toast('Evidence snapshot exported.');
};
const providerNames = {demo:'Built-in file agent',ollama:'Ollama',opencode:'OpenCode',codex:'Codex'};
let providerRevision = 0;
const providerConfig = () => ({kind:$('provider-kind').value,endpoint:$('provider-endpoint').value.trim(),model:$('provider-model').value.trim()});
function saveProvider() { try { localStorage.setItem('klyne-provider', JSON.stringify(providerConfig())); } catch (_) {} }
function configureProvider(config = {}) {
  const kind = $('provider-kind').value;
  ++providerRevision;
  $('provider-settings').hidden = kind === 'demo';
  $('provider-settings').open = kind !== 'demo';
  $('endpoint-field').hidden = kind === 'codex';
  $('provider-endpoint').disabled = kind === 'codex' || kind === 'demo';
  $('provider-endpoint').value = config.endpoint || ({ollama:'http://127.0.0.1:11434',opencode:'http://127.0.0.1:4096'}[kind] || '');
  $('provider-model').value = config.model || '';
  $('provider-model').required = kind === 'ollama' || kind === 'opencode';
  $('provider-model').disabled = kind === 'demo';
  $('model-optional').textContent = kind === 'codex' ? '(optional)' : '';
  $('provider-model').placeholder = kind === 'codex' ? 'Codex default model' : 'Check connection to find models';
  $('provider-help').textContent = ({ollama:'Uses models installed on this computer. Start Ollama, then check the connection.',opencode:'Start opencode serve --hostname 127.0.0.1 --port 4096. Model hosting depends on your OpenCode provider.',codex:'Uses your existing Codex CLI login. Sign in with codex login if needed. Model inference may use the cloud.'})[kind] || '';
  $('provider-status').textContent = 'Not checked';
  $('provider-summary').textContent = 'Configure';
  $('provider-models').replaceChildren();
  saveProvider();
}
$('provider-kind').onchange = () => configureProvider();
for (const id of ['provider-endpoint','provider-model']) $(id).oninput = () => { ++providerRevision; $('provider-status').textContent = 'Settings changed; check again.'; $('provider-summary').textContent = 'Configure'; saveProvider(); };
$('provider-model').addEventListener('invalid', () => { $('provider-settings').open = true; });
$('check-provider').onclick = async () => {
  const revision = providerRevision;
  $('check-provider').disabled = true; $('provider-status').textContent = 'Checking…';
  try {
    const result = await api('/api/connections/check', providerConfig());
    if (revision !== providerRevision) return;
    $('provider-status').textContent = result.message; $('provider-summary').textContent = 'Connected';
    $('provider-models').replaceChildren(...result.models.map(model => { const option = document.createElement('option'); option.value = model; return option; }));
    if (!$('provider-model').value && result.models.length) $('provider-model').value = result.models[0];
    saveProvider();
  } catch (error) { if (revision === providerRevision) { $('provider-status').textContent = error.message; $('provider-summary').textContent = 'Needs attention'; } }
  finally { $('check-provider').disabled = false; }
};
let savedProvider = {};
try { savedProvider = JSON.parse(localStorage.getItem('klyne-provider') || '{}') || {}; } catch (_) {}
if (Object.hasOwn(providerNames, savedProvider.kind)) $('provider-kind').value = savedProvider.kind;
configureProvider(savedProvider);
refresh(); setInterval(() => { if (!document.hidden) refresh(); }, 1500);
