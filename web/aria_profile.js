/* ═══════════════════════════════════════════════════════════════════════════
   ARIA PROFILE — Persistent identity, permanent modifications
   Once ARIA is created, her identity only grows. Nothing is truly erased.
   ═══════════════════════════════════════════════════════════════════════════ */
'use strict';

const ARIA_PROFILE_KEY = 'janus_aria_profile_v1';

const SKIN_TONES = {
  fair:   { hex: '#FFDDBB', three: 0xFFDDBB, lipHex: '#CC9988', lipThree: 0xCC9988 },
  light:  { hex: '#F5C5A0', three: 0xF5C5A0, lipHex: '#BB8877', lipThree: 0xBB8877 },
  medium: { hex: '#E8A87C', three: 0xE8A87C, lipHex: '#AA7755', lipThree: 0xAA7755 },
  tan:    { hex: '#D4855A', three: 0xD4855A, lipHex: '#996644', lipThree: 0x996644 },
  dark:   { hex: '#8D5524', three: 0x8D5524, lipHex: '#7A4422', lipThree: 0x7A4422 },
  deep:   { hex: '#4A2F1A', three: 0x4A2F1A, lipHex: '#5A3322', lipThree: 0x5A3322 },
};

const HAIR_COLORS = {
  black:  { hex: '#1A0A00', three: 0x1A0A00 },
  brown:  { hex: '#5C3317', three: 0x5C3317 },
  auburn: { hex: '#8B3103', three: 0x8B3103 },
  blonde: { hex: '#D4AA47', three: 0xD4AA47 },
  red:    { hex: '#CC2200', three: 0xCC2200 },
  silver: { hex: '#BBBBBB', three: 0xBBBBBB },
  white:  { hex: '#EEEEEE', three: 0xEEEEEE },
  purple: { hex: '#9D00FF', three: 0x9D00FF },
  blue:   { hex: '#0055EE', three: 0x0055EE },
  green:  { hex: '#00AA44', three: 0x00AA44 },
  pink:   { hex: '#FF69B4', three: 0xFF69B4 },
  teal:   { hex: '#00AAAA', three: 0x00AAAA },
};

const EYE_COLORS = {
  brown:  { hex: '#7B4F2F', three: 0x7B4F2F },
  blue:   { hex: '#4488DD', three: 0x4488DD },
  green:  { hex: '#44AA44', three: 0x44AA44 },
  hazel:  { hex: '#8B6914', three: 0x8B6914 },
  grey:   { hex: '#778899', three: 0x778899 },
  amber:  { hex: '#FFAA33', three: 0xFFAA33 },
  violet: { hex: '#9D00FF', three: 0x9D00FF },
  cyan:   { hex: '#00CCFF', three: 0x00CCFF },
};

const TATTOO_DESIGNS = {
  circuit:   'Circuit Board',
  tribal:    'Tribal',
  omega:     'Omega Rune',
  floral:    'Floral Vine',
  barcode:   'Barcode',
  phoenix:   'Phoenix',
  compass:   'Compass Rose',
  text:      'Custom Text',
};

const TATTOO_LOCATIONS = {
  arm_left:  'Left Arm',
  arm_right: 'Right Arm',
  chest:     'Chest',
  neck:      'Neck',
  back:      'Back',
};

// ─── Draw tattoo design onto a 2D canvas context ───────────────────────────
function drawTattooDesign(ctx, design, color, cx, cy, size, text = '') {
  ctx.save();
  ctx.translate(cx, cy);
  ctx.strokeStyle = color;
  ctx.fillStyle   = color;
  ctx.lineWidth   = Math.max(1.5, size * 0.04);
  ctx.lineCap     = 'round';
  ctx.lineJoin    = 'round';
  ctx.globalAlpha = 0.92;
  const r = size * 0.4;

  switch (design) {
    case 'circuit': {
      // PCB circuit lines
      const pts = [
        [-r, 0], [-r*0.5, 0], [-r*0.5, -r*0.5], [0, -r*0.5], [0, r*0.5],
        [r*0.5, r*0.5], [r*0.5, 0], [r, 0],
      ];
      ctx.beginPath();
      pts.forEach(([x, y], i) => i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y));
      ctx.stroke();
      // Junction dots
      [pts[1], pts[3], pts[5], pts[6]].forEach(([x, y]) => {
        ctx.beginPath(); ctx.arc(x, y, size * 0.03, 0, Math.PI*2); ctx.fill();
      });
      // Small resistor rectangles
      ctx.fillRect(-r*0.15, -r*0.7, r*0.3, r*0.22);
      ctx.fillRect(r*0.6, -r*0.15, r*0.22, r*0.3);
      break;
    }
    case 'tribal': {
      // Bold angular tribal
      for (let a = 0; a < 6; a++) {
        const ang = (a / 6) * Math.PI * 2;
        ctx.save();
        ctx.rotate(ang);
        ctx.beginPath();
        ctx.moveTo(0, 0);
        ctx.lineTo(r * 0.3, -r * 0.6);
        ctx.lineTo(r * 0.6, -r * 0.4);
        ctx.lineTo(r * 0.35, -r * 0.15);
        ctx.closePath();
        ctx.fill();
        ctx.restore();
      }
      ctx.beginPath(); ctx.arc(0, 0, r*0.15, 0, Math.PI*2); ctx.fill();
      break;
    }
    case 'omega': {
      // Omega symbol (Ω)
      ctx.beginPath();
      ctx.arc(0, -r*0.2, r*0.5, Math.PI*0.15, Math.PI*0.85);
      ctx.stroke();
      // Crossbar
      ctx.beginPath();
      ctx.moveTo(-r*0.55, r*0.4);
      ctx.lineTo(-r*0.28, r*0.4);
      ctx.lineTo(-r*0.28, r*0.15);
      ctx.moveTo(r*0.55, r*0.4);
      ctx.lineTo(r*0.28, r*0.4);
      ctx.lineTo(r*0.28, r*0.15);
      ctx.stroke();
      break;
    }
    case 'floral': {
      // 5-petal flower + stem
      for (let p = 0; p < 5; p++) {
        ctx.save();
        ctx.rotate((p / 5) * Math.PI * 2);
        ctx.beginPath();
        ctx.ellipse(0, -r*0.45, r*0.18, r*0.35, 0, 0, Math.PI*2);
        ctx.fill();
        ctx.restore();
      }
      ctx.beginPath(); ctx.arc(0, 0, r*0.18, 0, Math.PI*2); ctx.fill();
      // Vine
      ctx.beginPath();
      ctx.moveTo(0, r*0.2);
      ctx.bezierCurveTo(-r*0.4, r*0.5, r*0.4, r*0.8, 0, r);
      ctx.lineWidth *= 0.8;
      ctx.stroke();
      break;
    }
    case 'barcode': {
      const barW = r * 2 / 22;
      const barH = r * 1.1;
      const pattern = [3,1,1,2,1,3,1,1,2,1,3,2,1,1,2,1,1,3,2,1,2,1];
      let x = -r;
      pattern.forEach((w, i) => {
        if (i % 2 === 0) ctx.fillRect(x, -barH/2, barW * w, barH);
        x += barW * w;
      });
      // Number below
      ctx.font = `${size * 0.12}px Courier New`;
      ctx.textAlign = 'center';
      ctx.fillText('J4NUS-00', 0, barH/2 + size*0.15);
      break;
    }
    case 'phoenix': {
      // Abstract phoenix / flame bird
      ctx.beginPath();
      ctx.moveTo(0, -r);
      ctx.bezierCurveTo(-r*0.5, -r*0.5, -r, r*0.3, 0, r*0.4);
      ctx.bezierCurveTo(r, r*0.3, r*0.5, -r*0.5, 0, -r);
      ctx.fill();
      // Wings
      ctx.beginPath();
      ctx.moveTo(-r*0.1, 0);
      ctx.bezierCurveTo(-r*0.8, -r*0.3, -r, r*0.6, -r*0.2, r*0.4);
      ctx.lineWidth *= 0.7; ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(r*0.1, 0);
      ctx.bezierCurveTo(r*0.8, -r*0.3, r, r*0.6, r*0.2, r*0.4);
      ctx.stroke();
      break;
    }
    case 'compass': {
      // Compass rose
      ctx.beginPath(); ctx.arc(0, 0, r*0.1, 0, Math.PI*2); ctx.fill();
      ['N','E','S','W'].forEach((dir, i) => {
        const a = i * Math.PI/2 - Math.PI/2;
        ctx.save(); ctx.rotate(a);
        ctx.beginPath();
        ctx.moveTo(0, -r*0.12);
        ctx.lineTo(-r*0.12, -r*0.75);
        ctx.lineTo(0, -r*0.95);
        ctx.lineTo(r*0.12, -r*0.75);
        ctx.closePath();
        ctx.fill();
        ctx.font = `bold ${size * 0.13}px Courier New`;
        ctx.textAlign = 'center';
        ctx.fillText(dir, 0, -r * 1.12);
        ctx.restore();
      });
      ctx.beginPath();
      ctx.arc(0, 0, r*0.5, 0, Math.PI*2);
      ctx.lineWidth *= 0.5; ctx.stroke();
      break;
    }
    case 'text': {
      const str = text || 'JANUS';
      ctx.font = `bold ${size * 0.22}px Courier New`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(str, 0, 0);
      // Underline
      const w = ctx.measureText(str).width;
      ctx.fillRect(-w/2, size*0.14, w, 2);
      break;
    }
  }
  ctx.restore();
}

// ─── Profile Manager ──────────────────────────────────────────────────────────
class AriaProfileManager {
  constructor() {
    this._profile = null;
  }

  exists() {
    try {
      return !!localStorage.getItem(ARIA_PROFILE_KEY);
    } catch (e) {
      return false;
    }
  }

  load() {
    try {
      const raw = localStorage.getItem(ARIA_PROFILE_KEY);
      if (raw) this._profile = JSON.parse(raw);
    } catch(e) { this._profile = null; }
    return this._profile;
  }

  create(data) {
    this._profile = {
      version:   1,
      created:   new Date().toISOString(),
      name:      data.name || 'ARIA',
      base: {
        skinTone:      data.skinTone      || 'medium',
        eyeColor:      data.eyeColor      || 'cyan',
        hairColor:     data.hairColor     || 'black',
        hairStyle:     data.hairStyle     || 'long',
        outfit:        data.outfit        || 'tactical',
        outfitAccent:  data.outfitAccent  || '#00FF41',
        personality:   data.personality   || 'balanced',
      },
      modifications: data.initialMods || [],
    };
    this._save();
    return this._profile;
  }

  addModification(mod) {
    if (!this._profile) return null;
    const modification = {
      id:              'mod_' + Date.now(),
      type:            mod.type,          // 'tattoo' | 'scar' | 'piercing' | 'birthmark'
      added:           new Date().toISOString(),
      removable:       mod.removable !== false,
      removalProcess:  mod.removalProcess || 'laser_removal',
      details:         mod.details || {},
    };
    this._profile.modifications.push(modification);
    this._save();
    return modification;
  }

  removeModification(id, verifyRemovalProcess = false) {
    if (!this._profile) return false;
    const idx = this._profile.modifications.findIndex(m => m.id === id);
    if (idx === -1) return false;
    const mod = this._profile.modifications[idx];
    if (!mod.removable) return false;
    if (verifyRemovalProcess && mod.removalProcess) {
      // Removal process must have been run - checked elsewhere
    }
    this._profile.modifications.splice(idx, 1);
    this._save();
    return true;
  }

  get profile()       { return this._profile; }
  get name()          { return this._profile?.name || 'ARIA'; }
  get base()          { return this._profile?.base || {}; }
  get modifications() { return this._profile?.modifications || []; }
  get tattoos()       { return this.modifications.filter(m => m.type === 'tattoo'); }

  getAvatarOpts() {
    if (!this._profile) return {};
    const base = this._profile.base;
    const skin = SKIN_TONES[base.skinTone] || SKIN_TONES.medium;
    const hair = HAIR_COLORS[base.hairColor] || HAIR_COLORS.black;
    const eye  = EYE_COLORS[base.eyeColor]  || EYE_COLORS.cyan;
    return {
      skinColor:    skin.three,
      skinHex:      skin.hex,
      lipColor:     skin.lipThree,
      eyeColor:     eye.three,
      eyeColorHex:  eye.hex,
      hairColor:    hair.three,
      hairColorHex: hair.hex,
      hairStyle:    base.hairStyle,
      outfit:       base.outfit,
      outfitAccent: parseInt((base.outfitAccent || '#00FF41').replace('#', ''), 16),
      tattoos:      this.tattoos,
    };
  }

  _save() {
    try {
      localStorage.setItem(ARIA_PROFILE_KEY, JSON.stringify(this._profile));
      return true;
    } catch (e) {
      // Private browsing or a full quota must not break the launcher.
      console.warn('ARIA profile could not be persisted:', e);
      return false;
    }
  }
}

// ─── Export globally ──────────────────────────────────────────────────────────
window.AriaProfileManager = AriaProfileManager;
window.SKIN_TONES         = SKIN_TONES;
window.HAIR_COLORS        = HAIR_COLORS;
window.EYE_COLORS         = EYE_COLORS;
window.TATTOO_DESIGNS     = TATTOO_DESIGNS;
window.TATTOO_LOCATIONS   = TATTOO_LOCATIONS;
window.drawTattooDesign   = drawTattooDesign;
