/* ═══════════════════════════════════════════════════════════════════════════
   JANUS-OS — Main App Logic
   Integrates: ARIA Profile, Setup Wizard, 3D Avatar, WebSocket, Modules
   ═══════════════════════════════════════════════════════════════════════════ */
'use strict';

// ─── Profile ──────────────────────────────────────────────────────────────────
const pm = new AriaProfileManager();

// ─── WebSocket ─────────────────────────────────────────────────────────────────
let ws = null;
let wsReady  = false;
let currentCategory = null;
let opsCount = 0;

const wsQueue = [];

function setConnectionLabel(label) {
  const indicator = document.querySelector('.aria-online');
  if (indicator) indicator.textContent = label;
}

function connectWS() {
  if (ws && (ws.readyState === WebSocket.CONNECTING || ws.readyState === WebSocket.OPEN)) return;
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const socket = new WebSocket(`${protocol}//${location.host}/ws`);
  ws = socket;
  setConnectionLabel('● CONNECTING');
  socket.onopen = () => {
    wsReady = true;
    setConnectionLabel('● ONLINE');
    while (wsQueue.length && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify(wsQueue.shift()));
    }
  };
  socket.onmessage = (event) => {
    try {
      handleWsMessage(JSON.parse(event.data));
    } catch (error) {
      console.error('[JANUS] Invalid WebSocket message', error);
    }
  };
  socket.onclose = () => {
    if (ws !== socket) return;
    wsReady = false;
    setConnectionLabel('● OFFLINE · RECONNECTING');
    setTimeout(connectWS, 2000);
  };
  socket.onerror = () => socket.close();
}

function wsSend(obj) {
  const payload = JSON.stringify(obj);
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(payload);
  } else if (wsQueue.length < 100) {
    wsQueue.push(obj);
    connectWS();
  } else {
    appendTerminalLine('[ERROR] Connection queue is full. Please retry.');
  }
}

function handleWsMessage(msg) {
  switch (msg.type) {
    case 'aria_thought':
      updateAriaThought(msg.text);
      setAriaMood(moodFromText(msg.text));
      break;
    case 'aria_response':
      removeTyping();
      appendAriaChat('ARIA', msg.text);
      appendAriaViewChat('ARIA', msg.text);
      allAvatars(av => { av.speak(true); setTimeout(() => av.speak(false), 3500); });
      setAriaMood(moodFromText(msg.text));
      break;
    case 'module_list':
      renderModuleList(msg.modules || [], msg.category);
      break;
    case 'output':
      appendTerminalLine(msg.line || '');
      break;
    case 'module_complete':
      finishModule(false, msg.text);
      break;
    case 'module_error':
      finishModule(true, msg.text);
      break;
  }
}

function moodFromText(text = '') {
  const t = text.toLowerCase();
  if (/process|analys|calculat|scan/.test(t))           return 'processing';
  if (/alert|danger|threat|critical|warning/.test(t))   return 'excited';
  if (/curios|interest|wonder|hmm|intrigu/.test(t))     return 'curious';
  if (/success|complete|done|ready|finish/.test(t))     return 'happy';
  if (/fail|error|lost|disconnect|offline/.test(t))     return 'sad';
  return 'neutral';
}

// ─── Avatar instances ─────────────────────────────────────────────────────────
let avatarMain    = null;
let avatarChat    = null;
let avatarMini    = null;
let avatarProject = null;

function allAvatars(fn) {
  [avatarMain, avatarChat, avatarMini, avatarProject].forEach(av => av && fn(av));
}

function buildAvatarOpts(extra = {}) {
  const base = pm.exists() ? pm.getAvatarOpts() : {};
  return {
    skinColor:    0xE8A87C,
    lipColor:     0xAA7755,
    eyeColor:     0x00CCFF,
    hairColor:    0x1A0A00,
    outfitAccent: 0x00FF41,
    glowColor:    0x9D00FF,
    hairStyle:    'long',
    outfit:       'tactical',
    glowIntensity: 1.0,
    mood:         'curious',
    tattoos:      [],
    ...base,
    ...extra,
  };
}

function initMiniAvatar() {
  const canvas = document.getElementById('aria-mini-canvas');
  if (!canvas || !window.AriaAvatar3D || avatarMini) return;
  avatarMini = new AriaAvatar3D(canvas, buildAvatarOpts({ glowIntensity: 1.2 }));
  setAriaMood('curious');
}

function initChatAvatar() {
  const canvas = document.getElementById('aria-chat-canvas');
  if (!canvas || !window.AriaAvatar3D || avatarChat) return;
  avatarChat = new AriaAvatar3D(canvas, buildAvatarOpts({ glowIntensity: 1.4 }));
}

function initMainAvatar() {
  const canvas = document.getElementById('aria-main-canvas');
  if (!canvas || !window.AriaAvatar3D || avatarMain) return;
  const parent = document.getElementById('aria-3d-panel');
  if (parent) { canvas.width = parent.clientWidth; canvas.height = parent.clientHeight; }
  avatarMain = new AriaAvatar3D(canvas, buildAvatarOpts());
}

function initProjectionAvatar() {
  const canvas = document.getElementById('aria-project-canvas');
  if (!canvas || !window.AriaAvatar3D || avatarProject) return;
  canvas.width = window.innerWidth; canvas.height = window.innerHeight;
  avatarProject = new AriaAvatar3D(canvas, buildAvatarOpts({ glowIntensity: 1.8 }));
  avatarProject.projectLifeSize(true);
}

// ─── Mood ─────────────────────────────────────────────────────────────────────
function setAriaMood(mood) {
  allAvatars(av => av.setMood(mood));
  const names = { neutral:'◉ NEUTRAL', curious:'◉ CURIOUS', happy:'◉ HAPPY', excited:'⚡ EXCITED', processing:'⟳ PROCESSING', sad:'◎ REFLECTIVE' };
  const label  = names[mood] || '◉ NEUTRAL';
  const badge = document.getElementById('aria-mood-badge');
  if (badge) badge.textContent = label;
  const moodN = document.getElementById('aria-mood-name');
  if (moodN) moodN.textContent = label;
  document.querySelectorAll('.mood-btn').forEach(btn => btn.classList.toggle('active', btn.dataset.mood === mood));
}

// ─── Boot Sequence ─────────────────────────────────────────────────────────────
const BOOT_STEPS = [
  { pct:5,   label:'Loading kernel modules...' },
  { pct:12,  label:'Initialising udev device manager...' },
  { pct:20,  label:'Starting ADB server...',             ok:true },
  { pct:28,  label:'Mounting RAM filesystem...',         ok:true },
  { pct:36,  label:'Loading Janus security layer...',    ok:true },
  { pct:44,  label:'Initialising Lua runtime (54)...',   ok:true },
  { pct:52,  label:'Loading core modules (22)...',       ok:true },
  { pct:60,  label:'Loading plugin registry (10,626)...', ok:true },
  { pct:68,  label:'Starting Ghost-Net mesh...',         ok:true },
  { pct:76,  label:'Initialising Blackbox recorder...',  ok:true },
  { pct:84,  label:'Materialising ARIA holographic form...', ok:true },
  { pct:92,  label:"ARIA — online. I'm here.",           ok:true },
  { pct:100, label:'JanusOS ready.',                     ok:true },
];

function runBoot() {
  const progress = document.getElementById('boot-progress');
  const label    = document.getElementById('boot-label');
  const log      = document.getElementById('boot-log');
  const ariaMsg  = document.getElementById('aria-boot-msg');
  let i = 0;

  function tick() {
    if (i >= BOOT_STEPS.length) { setTimeout(afterBoot, 600); return; }
    const step = BOOT_STEPS[i++];
    progress.style.width = step.pct + '%';
    label.textContent = step.label;
    const line = document.createElement('div');
    line.className = 'log-line' + (step.ok ? ' ok' : '');
    line.textContent = `[${new Date().toISOString().substr(11,8)}] ${step.label}`;
    log.appendChild(line); log.scrollTop = log.scrollHeight;
    if (step.pct >= 84) ariaMsg.textContent = 'ARIA — holographic form materialising';
    if (step.pct >= 92) ariaMsg.textContent = 'ARIA — online and present';
    setTimeout(tick, step.ok ? 220 : 400);
  }
  tick();
}

function afterBoot() {
  document.getElementById('boot-screen').classList.add('hidden');
  // Check if ARIA has been set up
  pm.load();
  if (!pm.exists()) {
    // First time — run setup wizard
    const wizard = new AriaSetupWizard(pm, onSetupComplete);
  } else {
    showLauncher();
  }
}

function onSetupComplete(profile) {
  updateNameElements(profile.name);
  showLauncher();
}

function showLauncher() {
  document.getElementById('launcher').classList.remove('hidden');
  connectWS();
  startClock();
  updateNameElements(pm.name || 'ARIA');
  requestAnimationFrame(() => {
    initMiniAvatar();
    setAriaMood('curious');
    updateModBadges();
    updateModLog();
  });
  updateAriaThought('Systems online. Standing by, Operator.');
}

function updateNameElements(name) {
  ['aria-bar-name','aria-tile-name','dock-aria-name','aria-chat-panel-name',
   'aria-view-title-name','proj-title-name'].forEach(id => {
    const el = document.getElementById(id);
    if (el) el.textContent = name;
  });
  document.querySelectorAll('[id*="aria-name-badge"]').forEach(el => el.textContent = name);
}

// ─── Clock ─────────────────────────────────────────────────────────────────────
function startClock() {
  function tick() {
    const now = new Date();
    const t = document.getElementById('status-time');
    const d = document.getElementById('status-date');
    if (t) t.textContent = now.toTimeString().substr(0,5);
    if (d) d.textContent = now.toDateString().replace(/^\w+ /,'');
  }
  tick(); setInterval(tick, 1000);
}

// ─── ARIA thought bar ──────────────────────────────────────────────────────────
function updateAriaThought(text) {
  const el = document.getElementById('aria-bar-thought');
  if (!el) return;
  el.style.opacity = '0';
  setTimeout(() => { el.textContent = text; el.style.opacity = '1'; }, 280);
  const pt = document.getElementById('projection-thought');
  if (pt) { pt.style.opacity = '0'; setTimeout(() => { pt.textContent = `"${text}"`; pt.style.opacity = '1'; }, 280); }
}

function updateOpsCount() {
  const el = document.getElementById('ops-count');
  if (el) el.textContent = opsCount + ' ops';
}

// ─── App tiles ─────────────────────────────────────────────────────────────────
function openApp(category, label) {
  if (category === 'aria_avatar' || category === 'aria_chat') { openAriaView(); return; }
  currentCategory = category;
  document.getElementById('launcher').classList.add('hidden');
  const appView = document.getElementById('app-view');
  appView.classList.remove('hidden');

  appView.innerHTML = `
    <div id="app-header">
      <button id="app-back-btn" class="back-btn">◀ HOME</button>
      <div id="app-title">${esc(label || category)} — MODULE RUNNER</div>
      <div id="app-status" class="app-status-badge">LOADING</div>
    </div>
    <div class="app-body">
      <div id="module-list-panel">
        <div id="module-list-header">SELECT MODULE</div>
        <div id="module-list" class="module-list">
          <div style="padding:12px;font-size:0.6rem;color:var(--text3)">Loading modules...</div>
        </div>
      </div>
      <div id="terminal-panel">
        <div id="terminal-header">
          <span id="terminal-title">// terminal — ${esc(label || category)}</span>
          <button id="clear-btn" class="clear-btn">CLEAR</button>
        </div>
        <div id="terminal-output" class="terminal-output"></div>
        <div id="terminal-input-row">
          <span class="term-prompt">JANUS ❯</span>
          <input id="terminal-input" type="text" placeholder="Select a module above to execute..." autocomplete="off" spellcheck="false">
        </div>
      </div>
    </div>
  `;

  document.getElementById('app-back-btn').addEventListener('click', goHome);
  document.getElementById('clear-btn').addEventListener('click', clearTerminal);
  document.getElementById('terminal-input').addEventListener('keydown', onTerminalInput);

  appendTerminalLine('[JANUS] Loading module registry for: ' + (label || category).toUpperCase());
  appendTerminalLine('[' + (pm.name || 'ARIA') + '] Standing by. Select a module.');
  appendTerminalLine('');
  wsSend({ type: 'get_modules', category });
  document.getElementById('app-status').textContent = 'READY';
}

function goHome() {
  document.getElementById('app-view').classList.add('hidden');
  document.getElementById('aria-view').classList.add('hidden');
  document.getElementById('launcher').classList.remove('hidden');
  currentCategory = null;
}

// ─── Modules ───────────────────────────────────────────────────────────────────
function renderModuleList(modules, category) {
  const container = document.getElementById('module-list');
  if (!container) return;
  if (!modules.length) {
    container.innerHTML = '<div style="padding:12px;font-size:0.6rem;color:var(--text3)">No modules found.</div>';
    return;
  }
  container.innerHTML = '';
  modules.forEach(mod => {
    const item = document.createElement('div');
    item.className = 'module-item';
    item.innerHTML = `<div class="m-name">${esc(mod.display || mod.name)}</div><div class="m-desc">${esc(mod.description || 'Operational module')}</div>`;
    item.addEventListener('click', () => runModule(mod, category));
    container.appendChild(item);
  });
  const statusEl = document.getElementById('app-status');
  if (statusEl) statusEl.textContent = modules.length + ' MODULES';
  appendTerminalLine(`[REGISTRY] ${modules.length} modules loaded.`);
}

let _moduleRunning = false;

function runModule(mod, category) {
  if (_moduleRunning) return;
  _moduleRunning = true;
  const statusEl = document.getElementById('app-status');
  if (statusEl) { statusEl.textContent = 'RUNNING'; statusEl.className = 'app-status-badge running'; }
  appendTerminalLine('');
  appendTerminalLine(`[EXEC] Launching: ${mod.display || mod.name}`);
  wsSend({ type: 'run_module', category: category || currentCategory || '', module: mod.file || mod.name });
  setAriaMood('processing');
}

function finishModule(isError, label) {
  _moduleRunning = false;
  const statusEl = document.getElementById('app-status');
  if (isError) {
    if (statusEl) { statusEl.textContent = 'ERROR'; statusEl.className = 'app-status-badge error'; }
    appendTerminalLine(`[ERROR] Module '${label || ''}' failed.`);
    setAriaMood('sad');
    setTimeout(() => {
      if (statusEl) { statusEl.textContent = 'READY'; statusEl.className = 'app-status-badge'; }
      setAriaMood('neutral');
    }, 3000);
  } else {
    if (statusEl) { statusEl.textContent = 'READY'; statusEl.className = 'app-status-badge'; }
    opsCount++; updateOpsCount(); setAriaMood('happy');
    setTimeout(() => setAriaMood('curious'), 4000);
  }
}

// ─── Terminal ──────────────────────────────────────────────────────────────────
function appendTerminalLine(text) {
  const out = document.getElementById('terminal-output');
  if (!out) return;
  const div = document.createElement('div');
  if (!text.trim()) {
    div.className = 'term-line term-blank'; div.innerHTML = '&nbsp;';
  } else {
    const cls = text.includes('[' + (pm.name||'ARIA') + ']') || text.includes('ARIA:') ? ' aria-line'
              : text.includes('✓') || text.includes('[OK]') ? ' ok-line'
              : text.includes('WARN') ? ' warn-line'
              : /^[═─╔╚━]/.test(text) ? ' sep-line' : '';
    div.className = 'term-line' + cls;
    div.textContent = text;
  }
  out.appendChild(div); out.scrollTop = out.scrollHeight;
}

function clearTerminal() {
  const out = document.getElementById('terminal-output'); if (out) out.innerHTML = '';
}

function onTerminalInput(e) {
  if (e.key !== 'Enter') return;
  const input = document.getElementById('terminal-input');
  const val = input.value.trim(); if (!val) return;
  input.value = '';
  appendTerminalLine('JANUS ❯ ' + val);
  if (val === 'help')   appendTerminalLine('[HELP] Commands: clear, status, back, aria <msg>');
  else if (val === 'clear')  clearTerminal();
  else if (val === 'status') appendTerminalLine('[STATUS] JanusOS operational. ' + (pm.name||'ARIA') + ' online. ' + opsCount + ' ops.');
  else if (val === 'back')   goHome();
  else if (val.startsWith('aria ')) {
    const msg = val.slice(5);
    wsSend({ type: 'aria_chat', message: msg });
    appendTerminalLine('[YOU → ' + (pm.name||'ARIA') + '] ' + msg);
    setAriaMood('processing');
  } else appendTerminalLine('[JANUS] Unknown command. Type "help".');
}

// ─── ARIA Full View ────────────────────────────────────────────────────────────
function openAriaView() {
  document.getElementById('launcher').classList.add('hidden');
  document.getElementById('aria-view').classList.remove('hidden');
  requestAnimationFrame(() => {
    if (!avatarMain) initMainAvatar();
    setAriaMood('curious');
    updateModBadges();
    updateModLog();
  });
}

function closeAriaView() {
  document.getElementById('aria-view').classList.add('hidden');
  document.getElementById('launcher').classList.remove('hidden');
}

function appendAriaViewChat(from, text) {
  const msgs = document.getElementById('aria-view-messages');
  if (!msgs) return;
  const msg = document.createElement('div');
  msg.className = 'chat-msg ' + (from === (pm.name||'ARIA') ? 'aria-msg' : 'user-msg');
  msg.innerHTML = `<span class="chat-from">${esc(from)}</span><span class="chat-text">${esc(text)}</span>`;
  msgs.appendChild(msg); msgs.scrollTop = msgs.scrollHeight;
}

function sendAriaViewMessage() {
  const input = document.getElementById('aria-view-input');
  if (!input) return;
  const text = input.value.trim(); if (!text) return;
  input.value = '';
  appendAriaViewChat('YOU', text);
  wsSend({ type: 'aria_chat', message: text });
  setAriaMood('processing');
  showTyping('aria-view-messages', 'aria-view-typing');
}

// ─── Quick chat overlay ────────────────────────────────────────────────────────
function openAriaChat() {
  document.getElementById('aria-chat-overlay').classList.remove('hidden');
  requestAnimationFrame(() => {
    if (!avatarChat) initChatAvatar();
    setAriaMood('curious');
  });
  setTimeout(() => document.getElementById('aria-input')?.focus(), 280);
}

function closeAriaChat() {
  document.getElementById('aria-chat-overlay').classList.add('hidden');
}

function sendAriaMessage() {
  const input = document.getElementById('aria-input');
  const text = input.value.trim(); if (!text) return;
  input.value = '';
  appendAriaChat('YOU', text);
  wsSend({ type: 'aria_chat', message: text });
  setAriaMood('processing');
  showTyping('aria-chat-messages', 'aria-typing');
}

function appendAriaChat(from, text) {
  removeTyping();
  const msgs = document.getElementById('aria-chat-messages');
  if (!msgs) return;
  const msg = document.createElement('div');
  const ariaName = pm.name || 'ARIA';
  msg.className = 'chat-msg ' + (from === ariaName ? 'aria-msg' : 'user-msg');
  msg.innerHTML = `<span class="chat-from">${esc(from)}</span><span class="chat-text">${esc(text)}</span>`;
  msgs.appendChild(msg); msgs.scrollTop = msgs.scrollHeight;
}

function showTyping(containerId, typingId) {
  removeTyping();
  const msgs = document.getElementById(containerId);
  if (!msgs) return;
  const div = document.createElement('div');
  div.className = 'chat-msg aria-msg'; div.id = typingId;
  div.innerHTML = `<span class="chat-from">${esc(pm.name||'ARIA')}</span><span class="chat-text" style="color:var(--text3);font-style:italic">thinking...</span>`;
  msgs.appendChild(div); msgs.scrollTop = msgs.scrollHeight;
}

function removeTyping() {
  document.getElementById('aria-typing')?.remove();
  document.getElementById('aria-view-typing')?.remove();
}

// ─── Projection ────────────────────────────────────────────────────────────────
function openProjection() {
  document.getElementById('projection-overlay').classList.remove('hidden');
  requestAnimationFrame(() => {
    initProjectionAvatar();
    const thought = document.getElementById('aria-bar-thought')?.textContent || '';
    const pt = document.getElementById('projection-thought');
    if (pt && thought) pt.textContent = `"${thought}"`;
  });
}

function closeProjection() {
  document.getElementById('projection-overlay').classList.add('hidden');
  if (avatarProject) { avatarProject.dispose(); avatarProject = null; }
}

// ─── Permanent Marking System ─────────────────────────────────────────────────
function updateModBadges() {
  const container = document.getElementById('mod-badges');
  if (!container) return;
  container.innerHTML = '';
  pm.modifications.forEach(mod => {
    const badge = document.createElement('div');
    badge.className = 'mod-badge';
    badge.textContent = (mod.type.toUpperCase()) + ' · ' + (mod.details?.location || '').replace('_',' ').toUpperCase();
    container.appendChild(badge);
  });
}

function updateModLog() {
  const log = document.getElementById('mod-log');
  if (!log) return;
  if (!pm.modifications.length) {
    log.innerHTML = '<div style="font-size:0.52rem;color:var(--text3);padding:6px 0">No modifications yet.</div>';
    return;
  }
  log.innerHTML = '';
  pm.modifications.forEach(mod => {
    const entry = document.createElement('div');
    entry.className = 'mod-log-entry';
    const date = mod.added ? new Date(mod.added).toLocaleDateString() : '—';
    entry.innerHTML = `
      <span>
        <span class="ml-type">${mod.type.toUpperCase()}</span>
        <span> · ${(mod.details?.design || '').toUpperCase()} on ${(mod.details?.location || '').replace('_',' ').toUpperCase()}</span>
      </span>
      <span class="ml-date">${date}</span>
    `;
    log.appendChild(entry);
  });
}

function applyPermanentMarking() {
  const location = document.getElementById('mark-location')?.value || 'arm_left';
  const design   = document.getElementById('mark-design')?.value   || 'circuit';
  const color    = document.querySelector('#mark-colors .color-swatch.active')?.dataset.color || '#00FF41';
  const sizeVal  = document.getElementById('mark-size')?.value || 6;
  const text     = document.getElementById('mark-text')?.value || '';

  const mod = pm.addModification({
    type:           'tattoo',
    removable:      true,
    removalProcess: 'laser_removal',
    details: {
      location, design, color, text,
      size:    sizeVal / 10,
      posX:    0.5,
      posY:    location === 'neck' ? 0.25 : 0.38,
    },
  });

  // Apply to live avatar
  if (avatarMain) avatarMain.applyTattoo(mod);

  // Update UI
  updateModBadges();
  updateModLog();

  // Close panel
  document.getElementById('aria-marking-panel').classList.add('hidden');

  // Flash confirmation
  updateAriaThought('Marking applied. Permanent now.');
  setAriaMood('happy');
  setTimeout(() => setAriaMood('curious'), 4000);
}

// ─── Customisation panel ───────────────────────────────────────────────────────
function initCustomisationHandlers() {
  document.getElementById('customize-toggle-btn')?.addEventListener('click', () => {
    document.getElementById('aria-customize-panel').classList.toggle('hidden');
    document.getElementById('aria-marking-panel').classList.add('hidden');
  });

  document.getElementById('tattoo-btn')?.addEventListener('click', () => {
    document.getElementById('aria-marking-panel').classList.toggle('hidden');
    document.getElementById('aria-customize-panel').classList.add('hidden');
  });

  document.getElementById('mark-apply-btn')?.addEventListener('click', applyPermanentMarking);
  document.getElementById('mark-cancel-btn')?.addEventListener('click', () => {
    document.getElementById('aria-marking-panel').classList.add('hidden');
  });

  // Design → show/hide text field
  document.getElementById('mark-design')?.addEventListener('change', e => {
    const tf = document.getElementById('mark-text-row');
    if (tf) tf.style.display = e.target.value === 'text' ? 'block' : 'none';
  });

  // Size label
  const sizeRange = document.getElementById('mark-size');
  const sizeVal   = document.getElementById('mark-size-val');
  if (sizeRange) {
    sizeRange.addEventListener('input', () => {
      const v = parseInt(sizeRange.value);
      if (sizeVal) sizeVal.textContent = v <= 4 ? 'Small' : v <= 7 ? 'Medium' : 'Large';
    });
  }

  // Ink colour swatches
  document.querySelectorAll('#mark-colors .color-swatch').forEach(btn => {
    btn.addEventListener('click', () => {
      document.querySelectorAll('#mark-colors .color-swatch').forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
    });
  });

  // Mood buttons
  document.querySelectorAll('.mood-btn').forEach(btn => {
    btn.addEventListener('click', () => setAriaMood(btn.dataset.mood));
  });

  // Glow
  const glowRange = document.getElementById('glow-range');
  const glowVal   = document.getElementById('glow-val');
  if (glowRange) {
    glowRange.addEventListener('input', () => {
      const v = glowRange.value / 10;
      if (glowVal) glowVal.textContent = v.toFixed(1) + '×';
      allAvatars(av => { av.opts.glowIntensity = v; });
    });
  }

  // Height
  const heightRange = document.getElementById('height-range');
  const heightVal   = document.getElementById('height-val');
  if (heightRange) {
    heightRange.addEventListener('input', () => {
      const v = heightRange.value / 10;
      if (heightVal) heightVal.textContent = v.toFixed(1) + '×';
      allAvatars(av => av.customize({ height: v }));
    });
  }
}

// ─── Utility ───────────────────────────────────────────────────────────────────
function esc(str) {
  return String(str || '').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}

// ─── DOM ready ─────────────────────────────────────────────────────────────────
document.addEventListener('DOMContentLoaded', () => {

  // App tiles
  document.querySelectorAll('.app-tile').forEach(tile => {
    tile.addEventListener('click', () => openApp(tile.dataset.category, tile.dataset.label));
  });

  // Dock
  document.querySelectorAll('.dock-item').forEach(item => {
    item.addEventListener('click', () => {
      const cat   = item.dataset.category;
      const label = item.querySelector('span')?.textContent || cat;
      openApp(cat, label);
    });
  });

  // ARIA bar
  document.getElementById('aria-chat-btn')?.addEventListener('click', openAriaChat);
  document.getElementById('aria-bar-avatar-wrap')?.addEventListener('click', openAriaView);

  // Quick chat
  document.getElementById('aria-chat-close')?.addEventListener('click', closeAriaChat);
  document.getElementById('aria-send-btn')?.addEventListener('click', sendAriaMessage);
  document.getElementById('aria-input')?.addEventListener('keydown', e => { if (e.key==='Enter') sendAriaMessage(); });
  document.getElementById('aria-chat-overlay')?.addEventListener('click', e => {
    if (e.target.id === 'aria-chat-overlay') closeAriaChat();
  });

  // ARIA full view
  document.getElementById('aria-view-back')?.addEventListener('click', closeAriaView);
  document.getElementById('aria-view-send-btn')?.addEventListener('click', sendAriaViewMessage);
  document.getElementById('aria-view-input')?.addEventListener('keydown', e => { if (e.key==='Enter') sendAriaViewMessage(); });

  // Projection
  document.getElementById('project-btn')?.addEventListener('click', openProjection);
  document.getElementById('projection-exit-btn')?.addEventListener('click', closeProjection);
  document.getElementById('projection-overlay')?.addEventListener('click', e => {
    if (['projection-overlay','aria-project-canvas'].includes(e.target.id)) closeProjection();
  });

  // Customisation
  initCustomisationHandlers();

  // Boot
  runBoot();
});
