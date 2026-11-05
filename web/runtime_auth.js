/* Production authentication panel for the Janus runtime API. */
'use strict';

function runtimeAuthPanel() {
  const panel = document.createElement('section');
  panel.id = 'runtime-auth-panel';
  panel.style.cssText = 'position:fixed;right:16px;bottom:16px;z-index:10000;width:min(340px,calc(100vw - 32px));padding:14px;border:1px solid #00ff41;background:#07110a;color:#d8ffe1;font:12px monospace;box-shadow:0 0 22px #00ff4155;';
  panel.innerHTML = `
    <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:10px">
      <strong>JANUS RUNTIME ACCESS</strong><button type="button" data-action="hide" style="background:none;border:0;color:#9ab3a0">×</button>
    </div>
    <label style="display:block;margin-bottom:6px">Runtime URL <input data-field="url" style="width:100%;box-sizing:border-box" value="${escapeHtml(window.janusRuntime.baseUrl)}"></label>
    <label style="display:block;margin-bottom:6px">Email <input data-field="email" type="email" autocomplete="email" style="width:100%;box-sizing:border-box"></label>
    <label style="display:block;margin-bottom:8px">Password <input data-field="password" type="password" autocomplete="current-password" style="width:100%;box-sizing:border-box"></label>
    <label data-bootstrap style="display:none;margin-bottom:8px">Organization <input data-field="organization" style="width:100%;box-sizing:border-box"></label>
    <div style="display:flex;gap:8px"><button type="button" data-action="login">SIGN IN</button><button type="button" data-action="bootstrap">FIRST-TIME SETUP</button><button type="button" data-action="logout" style="display:none" data-logout>LOG OUT</button></div>
    <div data-status style="margin-top:9px;color:#9ab3a0">Not signed in. Demo display remains separate from production access.</div>`;
  document.body.appendChild(panel);

  const field = name => panel.querySelector(`[data-field="${name}"]`);
  const status = panel.querySelector('[data-status]');
  const bootstrap = panel.querySelector('[data-bootstrap]');
  const setStatus = (message, error = false) => { status.textContent = message; status.style.color = error ? '#ff7777' : '#9affaa'; };
  const signIn = async () => {
    try {
      window.janusRuntime.setBaseUrl(field('url').value.trim());
      const session = await window.janusRuntime.login(field('email').value, field('password').value);
      field('password').value = '';
      panel.querySelector('[data-logout]').style.display = 'inline-block';
      setStatus(`PRODUCTION SESSION: ${session.role}. Expires ${new Date(session.expires_at).toLocaleString()}.`);
      window.dispatchEvent(new CustomEvent('janus:session', { detail: session }));
    } catch (error) { setStatus(error.message, true); }
  };
  panel.addEventListener('click', async event => {
    const action = event.target.dataset.action;
    if (action === 'hide') { panel.style.display = 'none'; return; }
    if (action === 'logout') {
      try { await window.janusRuntime.logout(); } catch (_) {}
      panel.querySelector('[data-logout]').style.display = 'none';
      setStatus('Signed out. Production access is disabled.');
      window.dispatchEvent(new Event('janus:logout'));
      return;
    }
    if (action === 'bootstrap') {
      if (bootstrap.style.display === 'none') { bootstrap.style.display = 'block'; setStatus('Enter organization name, email, and a password of at least 12 characters.'); return; }
      try {
        window.janusRuntime.setBaseUrl(field('url').value.trim());
        const result = await window.janusRuntime.bootstrap(field('organization').value, field('email').value, field('password').value);
        setStatus(`Administrator created for organization ${result.organization_id}. Sign in now.`);
        bootstrap.style.display = 'none';
      } catch (error) { setStatus(error.message, true); }
      return;
    }
    if (action === 'login') await signIn();
  });

  // Load the deployment's configured runtime URL before restoring a session so
  // the dashboard works behind a reverse proxy without editing files.
  window.janusRuntime.loadServerConfig().then(() => {
    const urlField = field('url');
    if (urlField && document.activeElement !== urlField) urlField.value = window.janusRuntime.baseUrl;
    if (!window.janusRuntime.hasSession()) return;
    window.janusRuntime.me()
      .then(user => {
        panel.querySelector('[data-logout]').style.display = 'inline-block';
        setStatus(`Signed in as ${user.email} (${user.role}).`);
        window.dispatchEvent(new CustomEvent('janus:session', { detail: user }));
      })
      .catch(() => {
        window.janusRuntime.clearSession();
        setStatus('Stored session expired. Sign in again.', true);
      });
  });
}

function escapeHtml(value) {
  const node = document.createElement('span'); node.textContent = value; return node.innerHTML;
}

document.addEventListener('DOMContentLoaded', runtimeAuthPanel);
