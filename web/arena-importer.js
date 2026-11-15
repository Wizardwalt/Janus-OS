/* Janus Arena Chat Importer — all source processing happens in the browser. */
'use strict';

(() => {
  const MAX_INPUT_BYTES = 150 * 1024 * 1024;
  const MAX_JSON_CHARS = 30 * 1024 * 1024;
  const JSZIP_URL = 'https://cdn.jsdelivr.net/npm/jszip@3.10.1/dist/jszip.min.js';
  const state = { files: [], originals: [], conversations: [], warnings: [], imported: false };
  const $ = (id) => document.getElementById(id);

  function setStatus(message, kind = '') {
    const node = $('status');
    node.textContent = message;
    node.className = kind;
  }
  function addWarning(message) { state.warnings.push(message); }
  function safeName(name) {
    return String(name || 'arena-export').replace(/[^a-zA-Z0-9._-]+/g, '_').replace(/^\.+/, '').slice(0, 100) || 'arena-export';
  }
  function safeSlug(value) {
    return String(value || 'untitled').toLowerCase().normalize('NFKD')
      .replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 70) || 'untitled';
  }
  function getText(value) {
    if (value == null) return '';
    if (typeof value === 'string') return value;
    if (typeof value === 'number' || typeof value === 'boolean') return String(value);
    if (Array.isArray(value)) return value.map(getText).filter(Boolean).join('\n');
    if (typeof value === 'object') {
      if (typeof value.text === 'string') return value.text;
      if (typeof value.content === 'string') return value.content;
      if (Array.isArray(value.parts)) return getText(value.parts);
      if (Array.isArray(value.content)) return getText(value.content);
      if (typeof value.type === 'string' && /image|audio|video|file/i.test(value.type)) return `[${value.type} attachment]`;
      try { return JSON.stringify(value); } catch (_) { return '[unreadable content]'; }
    }
    return '';
  }
  function getEvaluation(record) {
    if (!record || typeof record !== 'object') return null;
    if (record.evaluation && typeof record.evaluation === 'object' && Array.isArray(record.evaluation.messages)) return record.evaluation;
    for (const key of ['conversation', 'chat']) {
      if (record[key] && typeof record[key] === 'object' && Array.isArray(record[key].messages)) return record[key];
    }
    if (Array.isArray(record.messages) && (record.id || record.title || record.conversationUrl || record.evaluationSessionId)) return record;
    return null;
  }
  function extractRecords(document, sourceName) {
    const found = [];
    const visit = (value, path = 'root') => {
      if (!value) return;
      if (Array.isArray(value)) { value.forEach((item, index) => visit(item, `${path}[${index}]`)); return; }
      if (typeof value !== 'object') return;
      const evaluation = getEvaluation(value);
      if (evaluation) { found.push({ wrapper: value, evaluation, sourceName, sourcePath: path }); return; }
      for (const key of ['evaluations', 'records', 'conversations', 'chats', 'items', 'data', 'results']) {
        if (value[key] && (Array.isArray(value[key]) || typeof value[key] === 'object')) visit(value[key], `${path}.${key}`);
      }
    };
    visit(document);
    return found;
  }
  function conversationKey(item, ordinal) {
    const e = item.evaluation;
    const id = e.id || e.evaluationSessionId || item.wrapper.id || item.wrapper.conversationUrl;
    return id ? `arena:${String(id)}` : `source:${item.sourceName}:${item.sourcePath}:${ordinal}`;
  }
  function normalizeRecord(item, ordinal) {
    const e = item.evaluation;
    const messages = Array.isArray(e.messages) ? e.messages : [];
    return {
      id: String(e.id || e.evaluationSessionId || item.wrapper.id || `${safeSlug(e.title)}-${ordinal}`),
      title: String(e.title || item.wrapper.title || 'Untitled Arena conversation'),
      source: item.wrapper.source || 'arena.ai (inferred from export format)',
      conversationUrl: e.conversationUrl || item.wrapper.conversationUrl || null,
      createdAt: e.createdAt || item.wrapper.createdAt || null,
      updatedAt: e.updatedAt || item.wrapper.updatedAt || null,
      messageCount: messages.length,
      messages,
      sourceFile: item.sourceName,
      sourcePath: item.sourcePath,
      rawRecord: item.wrapper,
      duplicateSources: []
    };
  }
  async function loadJsZip() {
    if (window.JSZip) return window.JSZip;
    await new Promise((resolve, reject) => {
      const script = document.createElement('script');
      script.src = JSZIP_URL;
      script.onload = resolve;
      script.onerror = () => reject(new Error('Could not load the ZIP reader. Connect to the internet and retry, or import the JSON files directly.'));
      document.head.appendChild(script);
    });
    if (!window.JSZip) throw new Error('ZIP reader did not initialize.');
    return window.JSZip;
  }
  async function processJsonText(text, sourceName) {
    if (text.length > MAX_JSON_CHARS) throw new Error(`${sourceName}: JSON file is too large (${(text.length / 1024 / 1024).toFixed(1)} MB).`);
    let parsed;
    try { parsed = JSON.parse(text.replace(/^\uFEFF/, '')); }
    catch (error) { throw new Error(`${sourceName}: invalid JSON (${error.message}).`); }
    const records = extractRecords(parsed, sourceName);
    if (!records.length) addWarning(`${sourceName}: valid JSON, but no Arena evaluation with a messages array was recognized. Original retained.`);
    records.forEach((record, index) => state.files.push({ record, ordinal: index }));
    return records.length;
  }
  async function readZip(file) {
    const JSZip = await loadJsZip();
    const zip = await JSZip.loadAsync(file, { checkCRC32: false });
    const entries = Object.values(zip.files).filter(entry => !entry.dir && entry.name.toLowerCase().endsWith('.json'));
    if (!entries.length) addWarning(`${file.name}: no JSON files found inside this ZIP. Original ZIP retained.`);
    let count = 0;
    let expandedBytes = 0;
    for (let index = 0; index < entries.length; index++) {
      const entry = entries[index];
      const declaredSize = entry._data && Number(entry._data.uncompressedSize);
      if (Number.isFinite(declaredSize) && declaredSize > MAX_JSON_CHARS) {
        addWarning(`${file.name}::${entry.name}: skipped because the uncompressed JSON entry exceeds the 30 MB safety limit.`);
        continue;
      }
      if (Number.isFinite(declaredSize)) expandedBytes += declaredSize;
      if (expandedBytes > 300 * 1024 * 1024) throw new Error(`${file.name}: ZIP expands beyond the 300 MB safety limit.`);
      const text = await entry.async('string');
      count += await processJsonText(text, `${safeName(file.name)}::${entry.name}`);
      $('progress').value = Math.round(((index + 1) / Math.max(entries.length, 1)) * 70);
    }
    return count;
  }
  function renderStats() {
    $('file-count').textContent = String(state.originals.length);
    $('chat-count').textContent = String(state.conversations.length);
    $('message-count').textContent = String(state.conversations.reduce((sum, c) => sum + c.messageCount, 0));
    const warningBox = $('warnings');
    warningBox.replaceChildren();
    if (state.warnings.length) {
      const heading = document.createElement('p'); heading.className = 'warn'; heading.textContent = `${state.warnings.length} note(s):`;
      const list = document.createElement('ul');
      state.warnings.forEach(warning => { const li = document.createElement('li'); li.textContent = warning; list.appendChild(li); });
      warningBox.append(heading, list);
    }
    const list = $('conversation-list'); list.replaceChildren();
    if (!state.conversations.length) {
      const li = document.createElement('li'); li.textContent = 'No conversations imported yet.'; list.appendChild(li);
    } else {
      state.conversations.slice().sort((a,b) => String(b.createdAt || '').localeCompare(String(a.createdAt || ''))).forEach(c => {
        const li = document.createElement('li');
        li.textContent = `${c.title} — ${c.messageCount} messages${c.createdAt ? ` — ${c.createdAt}` : ''}`;
        list.appendChild(li);
      });
    }
  }
  function markdownFor(conversation) {
    const lines = [
      `# ${conversation.title}`, '',
      `- Conversation ID: ${conversation.id}`,
      `- Created: ${conversation.createdAt || 'unknown'}`,
      `- Updated: ${conversation.updatedAt || 'unknown'}`,
      `- Source file: ${conversation.sourceFile}`,
      conversation.conversationUrl ? `- Arena URL: ${conversation.conversationUrl}` : '',
      '', '---', ''
    ];
    conversation.messages.forEach((message, index) => {
      const role = String(message.role || message.author || message.sender || 'unknown').toUpperCase();
      const time = message.createdAt || message.timestamp || '';
      const model = message.modelId || message.model || '';
      lines.push(`## ${index + 1}. ${role}${model ? ` · ${model}` : ''}`, time ? `_${time}_` : '', '',
        getText(message.content) || '[No text content; see raw JSON for structured attachments.]', '');
    });
    return lines.filter((line, index) => !(line === '' && lines[index - 1] === '')).join('\n');
  }
  function buildArchiveData() {
    return {
      schemaVersion: 1, archiveType: 'janus-arena-conversation-archive',
      generatedAt: new Date().toISOString(), source: 'arena.ai / LM Arena export files supplied by the user',
      preservation: 'Original source files are included separately. Each normalized conversation also retains its original rawRecord.',
      conversationCount: state.conversations.length,
      messageCount: state.conversations.reduce((sum, c) => sum + c.messageCount, 0),
      conversations: state.conversations
    };
  }
  function buildManifest() {
    return {
      schemaVersion: 1, generatedAt: new Date().toISOString(), importer: 'Janus Arena Chat Importer',
      originalFiles: state.originals.map(item => ({ name: item.name, type: item.type, size: item.size })),
      uniqueConversations: state.conversations.length,
      messages: state.conversations.reduce((sum, c) => sum + c.messageCount, 0),
      duplicateConversationRecords: state.conversations.reduce((sum, c) => sum + c.duplicateSources.length, 0),
      warnings: state.warnings
    };
  }
  function buildIndex() {
    const sorted = state.conversations.slice().sort((a,b) => String(a.createdAt || '').localeCompare(String(b.createdAt || '')));
    const lines = ['# Janus Arena Conversation Archive', '', `Generated: ${new Date().toISOString()}`, '',
      `- Unique conversations: ${sorted.length}`,
      `- Messages: ${sorted.reduce((sum,c) => sum + c.messageCount, 0)}`, '', '## Chronological index', ''];
    sorted.forEach((c, i) => lines.push(`${i + 1}. **${c.title}** — ${c.createdAt || 'date unknown'} — ${c.messageCount} messages — \`${safeSlug(c.id)}.md\``));
    return lines.join('\n');
  }
  async function makeZip() {
    const JSZip = await loadJsZip();
    const zip = new JSZip();
    state.originals.forEach((item, index) => {
      const name = `${String(index + 1).padStart(3, '0')}-${safeName(item.name)}`;
      zip.file(`originals/${name}`, item.data, { binary: true });
    });
    zip.file('janus-arena-archive.json', JSON.stringify(buildArchiveData(), null, 2));
    zip.file('manifest.json', JSON.stringify(buildManifest(), null, 2));
    zip.file('INDEX.md', buildIndex());
    state.conversations.forEach((conversation, index) => {
      const filename = `${String(index + 1).padStart(3, '0')}-${safeSlug(conversation.title)}-${safeSlug(conversation.id)}.md`;
      zip.file(`conversations/${filename}`, markdownFor(conversation));
    });
    return zip.generateAsync({ type: 'blob', compression: 'STORE' }, metadata => {
      $('progress').value = 70 + Math.round(metadata.percent * 0.3);
    });
  }
  function downloadBlob(blob, filename) {
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a'); anchor.href = url; anchor.download = filename; anchor.style.display = 'none';
    document.body.appendChild(anchor); anchor.click(); anchor.remove();
    setTimeout(() => URL.revokeObjectURL(url), 30000);
  }
  async function importSelected() {
    const selected = Array.from($('files').files || []);
    if (!selected.length) return;
    if (selected.some(file => file.size > MAX_INPUT_BYTES)) {
      setStatus('One or more files exceed the 150 MB per-file limit. Split the export into smaller batches.', 'error');
      return;
    }
    state.files = []; state.originals = []; state.conversations = []; state.warnings = [];
    $('progress').value = 0; $('import-btn').disabled = true;
    $('download-btn').disabled = true; $('json-btn').disabled = true;
    try {
      let totalRecords = 0;
      for (let i = 0; i < selected.length; i++) {
        const file = selected[i];
        setStatus(`Reading ${i + 1} of ${selected.length}: ${file.name}`);
        const bytes = await file.arrayBuffer();
        state.originals.push({ name: file.name, type: file.type || 'application/octet-stream', size: file.size, data: bytes });
        if (file.name.toLowerCase().endsWith('.zip') || file.type.includes('zip')) totalRecords += await readZip(file);
        else if (file.name.toLowerCase().endsWith('.json') || file.type.includes('json')) {
          totalRecords += await processJsonText(new TextDecoder().decode(bytes), file.name);
        } else addWarning(`${file.name}: unsupported file type; original retained.`);
        $('progress').value = Math.round(((i + 1) / selected.length) * 70);
        renderStats();
      }
      const seen = new Map();
      state.files.forEach((entry, index) => {
        const key = conversationKey(entry.record, index);
        const normalized = normalizeRecord(entry.record, index + 1);
        if (seen.has(key)) {
          const existing = seen.get(key);
          existing.duplicateSources.push({ sourceFile: normalized.sourceFile, sourcePath: normalized.sourcePath, messageCount: normalized.messageCount });
        } else {
          seen.set(key, normalized); state.conversations.push(normalized);
        }
      });
      state.imported = true;
      renderStats();
      $('download-btn').disabled = state.originals.length === 0;
      $('json-btn').disabled = state.conversations.length === 0;
      setStatus(`Import complete. Found ${totalRecords} conversation record(s), kept ${state.conversations.length} unique conversation(s), and preserved ${state.originals.length} original file(s).`, 'ok');
      $('progress').value = 100;
    } catch (error) {
      setStatus(error.message || String(error), 'error');
      addWarning(error.message || String(error)); renderStats();
    } finally { $('import-btn').disabled = false; }
  }
  $('files').addEventListener('change', () => {
    $('import-btn').disabled = !$('files').files.length;
    setStatus(`${$('files').files.length} file(s) selected. Ready to import.`);
  });
  $('import-btn').addEventListener('click', importSelected);
  $('reset-btn').addEventListener('click', () => {
    state.files = []; state.originals = []; state.conversations = []; state.warnings = []; state.imported = false;
    $('files').value = ''; $('progress').value = 0; $('import-btn').disabled = true;
    $('download-btn').disabled = true; $('json-btn').disabled = true;
    setStatus('Waiting for files.'); renderStats();
  });
  $('download-btn').addEventListener('click', async () => {
    try { setStatus('Building archive…'); const blob = await makeZip(); downloadBlob(blob, 'janus-arena-archive.zip'); setStatus('Archive ready. Check your Downloads folder.', 'ok'); }
    catch (error) { setStatus(error.message || String(error), 'error'); }
  });
  $('json-btn').addEventListener('click', () => {
    const blob = new Blob([JSON.stringify(buildArchiveData(), null, 2)], { type: 'application/json' });
    downloadBlob(blob, 'janus-arena-archive.json');
  });
  const drop = $('drop-zone');
  ['dragenter','dragover'].forEach(eventName => drop.addEventListener(eventName, event => { event.preventDefault(); drop.classList.add('drag'); }));
  ['dragleave','drop'].forEach(eventName => drop.addEventListener(eventName, event => { event.preventDefault(); drop.classList.remove('drag'); }));
  drop.addEventListener('drop', event => {
    const files = Array.from(event.dataTransfer.files || []);
    if (!files.length) return;
    const transfer = new DataTransfer(); files.forEach(file => transfer.items.add(file));
    $('files').files = transfer.files; $('import-btn').disabled = false;
    setStatus(`${files.length} file(s) selected. Ready to import.`);
  });
})();
