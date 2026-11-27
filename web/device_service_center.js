/* Janus Device Service Center — authenticated, read-only inventory.
 * This first increment performs discovery only; it never changes device state.
 */
'use strict';

function dscNode(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function dscCard(title, metric, detail) {
  const card = dscNode('section', 'device-service-card');
  card.appendChild(dscNode('h3', '', title));
  if (metric !== null) card.appendChild(dscNode('div', 'device-service-metric', String(metric)));
  if (detail) card.appendChild(dscNode('div', 'device-service-muted', detail));
  return card;
}

function openDeviceServiceCenter() {
  document.getElementById('launcher')?.classList.add('hidden');
  document.getElementById('aria-view')?.classList.add('hidden');
  const view = document.getElementById('app-view');
  if (!view) return;
  view.classList.remove('hidden');
  view.innerHTML = '<div id="app-header"><button id="device-service-back" class="back-btn" type="button">◀ HOME</button><div id="app-title">JANUS DEVICE SERVICE CENTER</div><div id="device-service-status" class="app-status-badge">READ-ONLY</div></div><div class="device-service-view"><div class="device-service-toolbar"><div class="device-service-muted">Connected hardware inventory · live runtime data only</div><button id="device-service-refresh" type="button">↻ REFRESH INVENTORY</button></div><div class="device-service-notice"><strong>SAFE DISCOVERY MODE</strong><br>This dashboard currently performs authenticated, read-only USB and serial discovery. No device is modified, unlocked, flashed, reset, or queried with protocol commands. Service actions will require separate authorization and confirmation.</div><div id="device-service-results" class="device-service-grid" aria-live="polite"><section class="device-service-card"><h3>DEVICE INVENTORY</h3><div class="device-service-muted">Loading verified runtime inventory…</div></section></div></div>';
  document.getElementById('device-service-back')?.addEventListener('click', goHome);
  document.getElementById('device-service-refresh')?.addEventListener('click', refreshDeviceServiceInventory);
  refreshDeviceServiceInventory();
}

async function refreshDeviceServiceInventory() {
  const results = document.getElementById('device-service-results');
  const status = document.getElementById('device-service-status');
  const refresh = document.getElementById('device-service-refresh');
  if (!results || !status) return;
  if (!window.janusRuntime?.hasSession()) {
    status.textContent = 'SIGN-IN REQUIRED';
    results.replaceChildren(dscCard('AUTHENTICATION REQUIRED', null, 'Sign in to Janus Runtime to view connected-device inventory. No simulated devices are displayed.'));
    return;
  }
  if (refresh) refresh.disabled = true;
  status.textContent = 'SCANNING INVENTORY';
  results.replaceChildren(dscCard('INVENTORY', null, 'Requesting read-only data from the authenticated Janus Runtime…'));
  const requests = await Promise.allSettled([
    window.janusRuntime.hardwareStatus(),
    window.janusRuntime.usbDevices(),
    window.janusRuntime.serialPorts(),
    window.janusRuntime.thermalSensors(),
    window.janusRuntime.memoryInventory()
  ]);
  results.replaceChildren();
  const hardware = requests[0], usb = requests[1], serial = requests[2], thermal = requests[3], memory = requests[4];
  function addCollection(title, result, formatter) {
    const items = result.status === 'fulfilled' && Array.isArray(result.value) ? result.value : [];
    const card = dscCard(title, result.status === 'fulfilled' ? items.length : '—',
      result.status === 'fulfilled' ? 'Detected by the runtime; limited to non-secret inventory metadata.' : 'Inventory endpoint unavailable.');
    if (result.status === 'fulfilled' && items.length) {
      const list = dscNode('div', 'device-service-list');
      items.forEach(item => list.appendChild(dscNode('div', 'device-service-device', formatter(item))));
      card.appendChild(list);
    } else if (result.status === 'fulfilled') {
      card.appendChild(dscNode('div', 'device-service-muted', 'No devices detected.'));
    } else {
      card.appendChild(dscNode('div', 'device-service-error', result.reason?.message || 'Request failed.'));
    }
    results.appendChild(card);
  }
  addCollection('USB DEVICES', usb, item => {
    const ids = [item.vendor_id, item.product_id].filter(v => v !== undefined && v !== null)
      .map(v => Number(v).toString(16).padStart(4, '0')).join(':');
    return [item.product || 'USB device', ids ? 'VID:PID ' + ids : null,
      item.manufacturer || null, item.bus_number ? 'Bus ' + item.bus_number : null].filter(Boolean).join(' · ');
  });
  addCollection('SERIAL PORTS', serial, item =>
    [item.port_name, item.transport, item.product, item.manufacturer].filter(Boolean).join(' · '));
  const adapters = hardware.status === 'fulfilled' && Array.isArray(hardware.value) ? hardware.value : [];
  const card = dscCard('HARDWARE ADAPTERS', hardware.status === 'fulfilled' ? adapters.length : '—',
    'Readiness reported by the existing hardware abstraction layer.');
  if (hardware.status === 'fulfilled') {
    const list = dscNode('div', 'device-service-list');
    adapters.forEach(item => {
      const row = dscNode('div', 'device-service-device');
      row.appendChild(dscNode('strong', item.state === 'available' ? 'device-service-state' : 'device-service-state unavailable',
        item.adapter + ': ' + item.state));
      row.appendChild(dscNode('div', 'device-service-muted', item.detail || 'No additional detail.'));
      list.appendChild(row);
    });
    if (!adapters.length) list.appendChild(dscNode('div', 'device-service-muted', 'No adapter status records returned.'));
    card.appendChild(list);
  } else {
    card.appendChild(dscNode('div', 'device-service-error', hardware.reason?.message || 'Request failed.'));
  }
  results.appendChild(card);
  const thermalItems = thermal.status === 'fulfilled' && Array.isArray(thermal.value) ? thermal.value : [];
  const thermalCard = dscCard('THERMAL SENSORS', thermal.status === 'fulfilled' ? thermalItems.length : '—',
    thermal.status === 'fulfilled' ? 'Read-only host thermal readings. Sensor availability depends on the runtime host.' : 'Thermal inventory unavailable.');
  if (thermal.status === 'fulfilled') {
    const list = dscNode('div', 'device-service-list');
    thermalItems.forEach(item => {
      const value = item.temperature_celsius === null || item.temperature_celsius === undefined
        ? 'Temperature unavailable' : Number(item.temperature_celsius).toFixed(1) + ' °C';
      list.appendChild(dscNode('div', 'device-service-device',
        [item.sensor_type || 'Sensor', item.sensor_id || null, value].filter(Boolean).join(' · ')));
    });
    if (!thermalItems.length) list.appendChild(dscNode('div', 'device-service-muted', 'No thermal sensors reported.'));
    thermalCard.appendChild(list);
  } else {
    thermalCard.appendChild(dscNode('div', 'device-service-error', thermal.reason?.message || 'Request failed.'));
  }
  results.appendChild(thermalCard);

  const memoryCard = dscCard('HOST MEMORY', null,
    memory.status === 'fulfilled' ? 'Read-only host memory inventory.' : 'Memory inventory unavailable.');
  if (memory.status === 'fulfilled' && memory.value) {
    const formatBytes = value => value === null || value === undefined
      ? 'Unavailable' : (Number(value) / (1024 ** 3)).toFixed(2) + ' GiB';
    const metrics = [
      ['Total', memory.value.total_bytes], ['Available', memory.value.available_bytes],
      ['Free', memory.value.free_bytes], ['Swap total', memory.value.swap_total_bytes],
      ['Swap free', memory.value.swap_free_bytes]
    ];
    const list = dscNode('div', 'device-service-list');
    metrics.forEach(([label, value]) => list.appendChild(dscNode('div', 'device-service-device', label + ': ' + formatBytes(value))));
    memoryCard.appendChild(list);
  } else if (memory.status === 'rejected') {
    memoryCard.appendChild(dscNode('div', 'device-service-error', memory.reason?.message || 'Request failed.'));
  } else {
    memoryCard.appendChild(dscNode('div', 'device-service-muted', 'No memory inventory returned.'));
  }
  results.appendChild(memoryCard);
  status.textContent = requests.some(r => r.status === 'rejected') ? 'PARTIAL INVENTORY' : 'LIVE · READ-ONLY';
  if (refresh) refresh.disabled = false;
}
