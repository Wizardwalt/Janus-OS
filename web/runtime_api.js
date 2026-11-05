/* Janus Runtime API client. Production views use this client instead of
   treating browser WebSocket output as proof that an operation ran.

   Base URL resolution order (first match wins):
     1. ?runtime=<url> query parameter (handy for one-off testing)
     2. janus.runtimeUrl in localStorage (set from the sign-in panel)
     3. window.JANUS_RUNTIME_URL injected by an embedding page
     4. /runtime-config.json served by janus-web (JANUS_RUNTIME_URL env var)
     5. /api — the same-origin path a reverse proxy forwards to the runtime
*/
'use strict';

const JANUS_DEFAULT_RUNTIME_URL = '/api';

class JanusRuntimeApi {
  constructor() {
    this.baseUrl = this._initialBaseUrl();
    this.token = sessionStorage.getItem('janus.sessionToken') || null;
  }

  _initialBaseUrl() {
    const queryValue = new URLSearchParams(window.location.search).get('runtime');
    const stored = localStorage.getItem('janus.runtimeUrl');
    return String(queryValue || stored || window.JANUS_RUNTIME_URL || JANUS_DEFAULT_RUNTIME_URL)
      .replace(/\/+$/, '');
  }

  /**
   * Ask the dashboard server which runtime URL this deployment uses. This lets
   * an operator point the UI at the protected runtime without editing files,
   * and keeps the browser on a single origin when a proxy is in front.
   */
  async loadServerConfig() {
    try {
      const response = await fetch('/runtime-config.json', {
        headers: { Accept: 'application/json' },
        cache: 'no-store',
      });
      if (!response.ok) return null;
      const config = await response.json();
      const override = localStorage.getItem('janus.runtimeUrl')
        || new URLSearchParams(window.location.search).get('runtime');
      if (!override && config && typeof config.runtime_url === 'string' && config.runtime_url) {
        this.baseUrl = config.runtime_url.replace(/\/+$/, '');
      }
      return config;
    } catch (_) {
      return null;
    }
  }

  setBaseUrl(url) {
    this.baseUrl = String(url || '').trim().replace(/\/+$/, '');
    if (this.baseUrl) {
      localStorage.setItem('janus.runtimeUrl', this.baseUrl);
    } else {
      localStorage.removeItem('janus.runtimeUrl');
      this.baseUrl = JANUS_DEFAULT_RUNTIME_URL;
    }
  }

  hasSession() { return Boolean(this.token); }

  async request(path, options = {}) {
    const headers = { 'Content-Type': 'application/json', ...(options.headers || {}) };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    let response;
    try {
      response = await fetch(`${this.baseUrl}${path}`, { ...options, headers });
    } catch (cause) {
      const error = new Error(`Runtime unreachable at ${this.baseUrl || window.location.origin}. Check the runtime URL and that the service is running.`);
      error.cause = cause;
      throw error;
    }
    const body = await response.json().catch(() => ({ status: 'error', error: 'Invalid runtime response' }));
    if (!response.ok || body.status === 'error') {
      if (response.status === 401) this.clearSession();
      const error = new Error(body.error || `Runtime request failed (${response.status})`);
      error.status = response.status;
      throw error;
    }
    return body.data;
  }

  clearSession() {
    this.token = null;
    sessionStorage.removeItem('janus.sessionToken');
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
    try {
      if (this.token) await this.request('/auth/logout', { method: 'POST' });
    } finally {
      this.clearSession();
    }
  }

  health() { return this.request('/health'); }
  me() { return this.request('/auth/me'); }
  engagements() { return this.request('/engagements'); }
  licenseStatus() { return this.request('/licenses/status'); }
  auditLogs() { return this.request('/audit/logs'); }
  plugins() { return this.request('/plugins'); }
  hardwareStatus() { return this.request('/hardware/status'); }
  users() { return this.request('/auth/users'); }
  setUserActive(userId, active) { return this.request(`/auth/users/${encodeURIComponent(userId)}/${active ? 'enable' : 'disable'}`, { method: 'POST' }); }
  resetUserPassword(userId, password) { return this.request(`/auth/users/${encodeURIComponent(userId)}/password-reset`, { method: 'POST', body: JSON.stringify({ password }) }); }
  setEngagementActive(id, active) { return this.request(`/engagements/${encodeURIComponent(id)}/${active ? 'enable' : 'disable'}`, { method: 'POST' }); }

  execute(plugin, engagementId, targetAsset, args = {}) {
    return this.request('/execute', {
      method: 'POST',
      body: JSON.stringify({ plugin, engagement_id: engagementId, target_asset: targetAsset, args }),
    });
  }
}

window.janusRuntime = new JanusRuntimeApi();
