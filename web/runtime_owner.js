/* Purchaser device ownership and configurable security policy console. */
'use strict';

function createOwnerConsole() {
  const button = document.createElement('button');
  button.id = 'owner-console-open';
  button.type = 'button';
  button.textContent = 'OWNER SECURITY';
  button.style.cssText = 'position:fixed;left:16px;bottom:16px;z-index:9997;padding:10px 12px;border:1px solid #00ff41;background:#07110a;color:#baffc7;font:11px monospace;display:none';
  document.body.appendChild(button);

  const panel = document.createElement('section');
  panel.id = 'owner-console';
  panel.style.cssText = 'position:fixed;inset:8% 5%;z-index:10001;overflow:auto;padding:16px;border:1px solid #00ff41;background:#07110af7;color:#d8ffe1;font:12px monospace;display:none;box-shadow:0 0 28px #00ff4155';
  const title = document.createElement('h2');
  title.textContent = 'OWNER SECURITY CONSOLE';
  const close = document.createElement('button');
  close.type = 'button'; close.textContent = 'CLOSE';
  close.style.cssText = 'float:right';
  const status = document.createElement('p');
  status.setAttribute('role', 'status');
  status.style.cssText = 'margin:12px 0;color:#9ab3a0';
  const register = document.createElement('div');
  register.style.cssText = 'padding:12px;margin:12px 0;border:1px solid #315b3a';
  const registerTitle = document.createElement('h3'); registerTitle.textContent = 'REGISTER A DEVICE';
  const deviceId = document.createElement('input');
  deviceId.placeholder = 'Device ID (8-128 characters)';
  deviceId.autocomplete = 'off';
  const deviceType = document.createElement('select');
  [['monolith','Monolith'],['pipboy','Pip-Boy']].forEach(([value,label]) => {
    const option = document.createElement('option'); option.value = value; option.textContent = label; deviceType.appendChild(option);
  });
  const registerPassword = document.createElement('input');
  registerPassword.type = 'password'; registerPassword.autocomplete = 'current-password'; registerPassword.placeholder = 'Re-enter owner password';
  const registerButton = document.createElement('button'); registerButton.type = 'button'; registerButton.textContent = 'REGISTER DEVICE';
  [deviceId, deviceType, registerPassword, registerButton].forEach(el => {
    el.style.cssText = 'display:block;width:100%;max-width:480px;margin:7px 0;padding:8px;background:#101c13;color:#d8ffe1;border:1px solid #315b3a;font:12px monospace';
    register.appendChild(el);
  });
  register.insertBefore(registerTitle, deviceId);
  const list = document.createElement('div');
  panel.append(title, close, status, register, list);
  document.body.appendChild(panel);

  const keys = [
    ['require_approval_for_sensitive','Require approval for sensitive actions'],
    ['remote_access_enabled','Allow remote access'],
    ['cloud_provider_enabled','Allow cloud model providers'],
    ['microphone_enabled','Allow microphone access'],
    ['hardware_modules_enabled','Allow hardware modules'],
    ['strict_mode','Strict security mode'],
  ];
  const setStatus = (message, error = false) => {
    status.textContent = message;
    status.style.color = error ? '#ff7777' : '#9affaa';
  };

  async function refresh() {
    if (!window.janusRuntime?.hasSession()) { button.style.display = 'none'; panel.style.display = 'none'; return; }
    try {
      const user = await window.janusRuntime.me();
      const devices = await window.janusRuntime.ownedDevices();
      button.style.display = 'block';
      register.style.display = user.role === 'organization_admin' ? 'block' : 'none';
      list.replaceChildren();
      if (!devices.length) {
        const empty = document.createElement('p'); empty.textContent = 'No devices are registered to this account yet.'; list.appendChild(empty);
      }
      devices.forEach(device => {
        const card = document.createElement('article');
        card.style.cssText = 'margin:12px 0;padding:12px;border:1px solid #315b3a';
        const heading = document.createElement('h3');
        heading.textContent = device.device_type.toUpperCase() + ' · ' + device.device_id;
        card.appendChild(heading);
        const fields = {};
        keys.forEach(([key,label]) => {
          const row = document.createElement('label');
          row.style.cssText = 'display:flex;align-items:center;justify-content:space-between;gap:12px;margin:9px 0';
          const text = document.createElement('span'); text.textContent = label;
          const checkbox = document.createElement('input');
          checkbox.type = 'checkbox'; checkbox.checked = Boolean(device.policies[key]); checkbox.dataset.policy = key;
          fields[key] = checkbox; row.append(text, checkbox); card.appendChild(row);
        });
        const password = document.createElement('input');
        password.type = 'password'; password.autocomplete = 'current-password'; password.placeholder = 'Re-enter owner password to save';
        password.style.cssText = 'display:block;width:100%;max-width:480px;margin:8px 0;padding:8px;background:#101c13;color:#d8ffe1;border:1px solid #315b3a';
        const save = document.createElement('button'); save.type = 'button'; save.textContent = 'SAVE SECURITY SETTINGS';
        save.addEventListener('click', async () => {
          const policies = Object.fromEntries(keys.map(([key]) => [key, fields[key].checked]));
          const weakens = ['remote_access_enabled','cloud_provider_enabled','microphone_enabled','hardware_modules_enabled']
            .some(key => policies[key] && !device.policies[key]) ||
            (device.policies.strict_mode && !policies.strict_mode) ||
            (device.policies.require_approval_for_sensitive && !policies.require_approval_for_sensitive);
          if (weakens && !window.confirm('These changes reduce one or more protections. Continue?')) return;
          save.disabled = true;
          try {
            const saved = await window.janusRuntime.updateOwnedDevicePolicy(device.device_id, password.value, policies);
            password.value = '';
            device.policies = saved;
            setStatus('Policy record saved. Runtime enforcement is still being integrated; these settings do not yet control every subsystem.');
            await refresh();
          } catch (error) { setStatus(error.message, true); }
          finally { save.disabled = false; }
        });
        card.append(password, save);
        list.appendChild(card);
      });
      setStatus('Signed in as ' + user.email + '. Policy changes require password re-entry and are audited.');
    } catch (error) {
      button.style.display = 'block';
      setStatus('Owner console unavailable: ' + error.message, true);
    }
  }

  button.addEventListener('click', async () => { panel.style.display = 'block'; await refresh(); });
  close.addEventListener('click', () => { panel.style.display = 'none'; });
  registerButton.addEventListener('click', async () => {
    registerButton.disabled = true;
    try {
      const result = await window.janusRuntime.registerOwnedDevice(deviceId.value.trim(), deviceType.value, registerPassword.value);
      deviceId.value = ''; registerPassword.value = '';
      setStatus('Device registered to this account: ' + result.device_id);
      await refresh();
    } catch (error) { setStatus(error.message, true); }
    finally { registerButton.disabled = false; }
  });
  window.addEventListener('janus:session', refresh);
  window.addEventListener('janus:logout', () => { button.style.display = 'none'; panel.style.display = 'none'; list.replaceChildren(); });
  if (window.janusRuntime?.hasSession()) refresh();
}

document.addEventListener('DOMContentLoaded', createOwnerConsole);
