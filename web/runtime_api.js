/* Janus Runtime API client. Production views use this client instead of
   treating browser WebSocket output as proof that an operation ran. */
'use strict';

class JanusRuntimeApi {
  constructor() {
    const savedUrl = localStorage.getItem('janus.runtimeUrl');
    // Migrate the old localhost default: in hosted browsers, localhost refers
    // to the operator's device, not the Janus server. /api is same-origin.
    this.baseUrl = !savedUrl || savedUrl === 'http://localhost:8080'
      ? `${window.location.origin}/api`
      : savedUrl;
    if (this.baseUrl !== savedUrl) localStorage.setItem('janus.runtimeUrl', this.baseUrl);
    this.token = sessionStorage.getItem('janus.sessionToken') || null;
  }

  setBaseUrl(url) {
    this.baseUrl = url.replace(/\/$/, '');
    localStorage.setItem('janus.runtimeUrl', this.baseUrl);
  }

  hasSession() { return Boolean(this.token); }

  async request(path, options = {}) {
    const headers = { 'Content-Type': 'application/json', ...(options.headers || {}) };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    const response = await fetch(`${this.baseUrl}${path}`, { ...options, headers });
    const body = await response.json().catch(() => ({ status: 'error', error: 'Invalid runtime response' }));
    if (!response.ok || body.status === 'error') {
      const error = new Error(body.error || `Runtime request failed (${response.status})`);
      error.status = response.status;
      throw error;
    }
    return body.data;
  }

  async bootstrap(organizationName, email, password) {
    return this.request('/auth/bootstrap', {
      method: 'POST', body: JSON.stringify({ organization_name: organizationName, email, password }),
    });
  }

  async login(email, password) {
    const session = await this.request('/auth/login', {
      method: 'POST', body: JSON.stringify({ email, password }),
    });
    this.token = session.session_token;
    sessionStorage.setItem('janus.sessionToken', this.token);
    return session;
  }

  async logout() {
    if (this.token) await this.request('/auth/logout', { method: 'POST' });
    this.token = null;
    sessionStorage.removeItem('janus.sessionToken');
  }

  me() { return this.request('/auth/me'); }
  engagements() { return this.request('/engagements'); }
  licenseStatus() { return this.request('/licenses/status'); }
  auditLogs() { return this.request('/audit/logs'); }
  getState(namespace, key) { return this.request(`/state/${encodeURIComponent(namespace)}/${encodeURIComponent(key)}`); }
  setState(namespace, key, value) { return this.request(`/state/${encodeURIComponent(namespace)}/${encodeURIComponent(key)}`, { method: 'POST', body: JSON.stringify(value) }); }
  plugins() { return this.request('/plugins'); }
  hardwareStatus() { return this.request('/hardware/status'); }

  // Aria knowledge, encrypted history, and provider-backed chat APIs. The
  // legacy WebSocket path remains explicitly labeled as scripted demo mode.
  ariaImportText(title, text) {
    return this.request('/aria/artifacts/text', {
      method: 'POST', body: JSON.stringify({ title, text }),
    });
  }
  ariaSearch(query, limit = 20) {
    const params = new URLSearchParams({ q: query, limit: String(limit) });
    return this.request(`/aria/search?${params.toString()}`);
  }
  ariaStatus() {
    return this.request('/aria/status');
  }
  ariaChat(conversationId, message, requestId = null, includePrivateContext = false) {
    const id = requestId || window.crypto?.randomUUID?.()
      || `chat-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    return this.request('/aria/chat', {
      method: 'POST',
      body: JSON.stringify({
        conversation_id: conversationId,
        message,
        request_id: id,
        include_private_context: Boolean(includePrivateContext),
      }),
    });
  }
  ariaConversations(limit = 20) {
    return this.request(`/aria/conversations?limit=${encodeURIComponent(String(limit))}`);
  }
  ariaConversationEvents(conversationId, limit = 100) {
    return this.request(`/aria/conversations/${encodeURIComponent(conversationId)}/events?limit=${encodeURIComponent(String(limit))}`);
  }
  ariaDeleteConversation(conversationId) {
    return this.request(`/aria/conversations/${encodeURIComponent(conversationId)}`, { method: 'DELETE' });
  }
  ariaRemember(content, sourceRefs = [], expiresAt = null) {
    return this.request('/aria/memory', {
      method: 'POST',
      body: JSON.stringify({ content, source_refs: sourceRefs, expires_at: expiresAt }),
    });
  }
  ariaMemories() { return this.request('/aria/memory'); }
  ariaCorrectMemory(memoryId, content, sourceRefs = []) {
    return this.request(`/aria/memory/${encodeURIComponent(memoryId)}/correct`, {
      method: 'POST', body: JSON.stringify({ content, source_refs: sourceRefs }),
    });
  }
  ariaForgetMemory(memoryId) {
    return this.request(`/aria/memory/${encodeURIComponent(memoryId)}`, { method: 'DELETE' });
  }
  ariaCreateProposal(artifactId, query, title, hatIds, tags = []) {
    return this.request('/aria/proposals', {
      method: 'POST', body: JSON.stringify({ artifact_id: artifactId, query, title, hat_ids: hatIds, tags }),
    });
  }
  ariaPendingProposals() { return this.request('/aria/proposals/pending'); }
  ariaReviewProposal(proposalId, decision, notes = '') {
    return this.request(`/aria/proposals/${encodeURIComponent(proposalId)}/review`, {
      method: 'POST', body: JSON.stringify({ decision, notes }),
    });
  }
  ariaPersistLearning() {
    return this.request('/aria/learning/persist', { method: 'POST' });
  }
  users() { return this.request('/auth/users'); }
  setUserActive(userId, active) { return this.request(`/auth/users/${encodeURIComponent(userId)}/${active ? 'enable' : 'disable'}`, { method: 'POST' }); }
  resetUserPassword(userId, password) { return this.request(`/auth/users/${encodeURIComponent(userId)}/password-reset`, { method: 'POST', body: JSON.stringify({ password }) }); }
  setEngagementActive(id, active) { return this.request(`/engagements/${encodeURIComponent(id)}/${active ? 'enable' : 'disable'}`, { method: 'POST' }); }

  ownedDevices() { return this.request('/owner/devices'); }
  registerOwnedDevice(deviceId, deviceType, password) {
    return this.request('/owner/devices', {
      method: 'POST', body: JSON.stringify({ device_id: deviceId, device_type: deviceType, password }),
    });
  }
  updateOwnedDevicePolicy(deviceId, password, policies) {
    return this.request(`/owner/devices/${encodeURIComponent(deviceId)}/policies`, {
      method: 'POST', body: JSON.stringify({ password, policies }),
    });
  }

  execute(plugin, engagementId, targetAsset, args = {}) {
    return this.request('/execute', {
      method: 'POST',
      body: JSON.stringify({ plugin, engagement_id: engagementId, target_asset: targetAsset, args }),
    });
  }
}

window.janusRuntime = new JanusRuntimeApi();
