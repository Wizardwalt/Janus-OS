-- =============================================================================
-- JANUS VOICE SYSTEM — ARIA Speaks
-- Text-to-speech with personality-matched voice profiles and emotion inflection
-- =============================================================================

local voice = {}

-- ─── VOICE PROFILES ───────────────────────────────────────────────────────────
-- Mapped to espeak/piper parameters
voice.profiles = {
    Oracle  = { engine="espeak", voice="en-us+f3", speed=145, pitch=52, amplitude=85,
                desc="Calm, measured, slightly ethereal female voice" },
    Ghost   = { engine="espeak", voice="en-us+m3", speed=130, pitch=35, amplitude=60,
                desc="Low, quiet, slightly distorted — sparse whisper" },
    Titan   = { engine="espeak", voice="en-us+m1", speed=165, pitch=25, amplitude=100,
                desc="Deep, powerful, commanding male voice" },
    Scholar = { engine="espeak", voice="en-us+f4", speed=155, pitch=58, amplitude=80,
                desc="Clear, precise, slightly faster — articulate" },
    Renegade= { engine="espeak", voice="en-us+f2", speed=180, pitch=65, amplitude=95,
                desc="Fast, energetic, punchy — expressive" },
    Phantom = { engine="espeak", voice="en-us+f4", speed=120, pitch=60, amplitude=70,
                desc="Slow, dreamy, slightly reverberant" },
}

-- ─── EMOTION VOICE MODIFIERS ──────────────────────────────────────────────────
voice.emotion_mods = {
    curious    = { speed_mod=0,   pitch_mod=5,  amp_mod=0   },
    focused    = { speed_mod=-10, pitch_mod=-5,  amp_mod=-10 },
    alert      = { speed_mod=20, pitch_mod=10, amp_mod=15 },
    excited    = { speed_mod=25, pitch_mod=15, amp_mod=15 },
    tired      = { speed_mod=-25, pitch_mod=-10, amp_mod=-20 },
    satisfied  = { speed_mod=-5,  pitch_mod=0,   amp_mod=-5  },
    concerned  = { speed_mod=5,  pitch_mod=5,  amp_mod=5  },
    proud      = { speed_mod=0,   pitch_mod=-5,  amp_mod=10 },
    bored      = { speed_mod=-20, pitch_mod=-15, amp_mod=-25 },
    playful    = { speed_mod=15, pitch_mod=20, amp_mod=10 },
    protective = { speed_mod=10, pitch_mod=-8,  amp_mod=20 },
    melancholic= { speed_mod=-20, pitch_mod=-10, amp_mod=-20 },
    loving     = { speed_mod=-10, pitch_mod=10, amp_mod=-5  },
    determined = { speed_mod=5,  pitch_mod=-5,  amp_mod=15 },
}

-- ─── STATE ────────────────────────────────────────────────────────────────────
voice.state = {
    enabled          = true,
    current_archetype= "Oracle",
    current_emotion  = "curious",
    stealth_mode     = false,      -- whisper mode — very quiet
    volume_percent   = 80,
    lines_spoken     = 0,
    last_phrase      = "",
}

-- ─── CORE SPEAK FUNCTION ──────────────────────────────────────────────────────
function voice.speak(text, archetype, emotion)
    if not voice.state.enabled then
        janus.log("[VOICE] Voice disabled. Text: " .. text)
        return
    end

    archetype = archetype or voice.state.current_archetype
    emotion   = emotion   or voice.state.current_emotion

    local profile = voice.profiles[archetype] or voice.profiles["Oracle"]
    local mod     = voice.emotion_mods[emotion] or { speed_mod=0, pitch_mod=0, amp_mod=0 }

    local speed = math.max(80, math.min(250, profile.speed + mod.speed_mod))
    local pitch = math.max(0,  math.min(99,  profile.pitch + mod.pitch_mod))
    local amp   = math.max(10, math.min(100, profile.amplitude + mod.amp_mod))

    -- Stealth mode: drop amplitude to whisper
    if voice.state.stealth_mode then
        amp = math.min(amp, 25)
        speed = math.max(80, speed - 20)
    end

    -- Build espeak command
    local escaped = text:gsub("'", "'\\''"):gsub('"', '\\"')
    local cmd = string.format(
        "espeak -v %s -s %d -p %d -a %d '%s' 2>/dev/null &",
        profile.voice, speed, pitch, amp, escaped
    )

    local result = janus.shell("command -v espeak 2>/dev/null")
    if result and result ~= "" and not result:find("not found") then
        janus.shell(cmd)
        voice.state.lines_spoken = voice.state.lines_spoken + 1
        voice.state.last_phrase  = text
        janus.log(string.format("[VOICE] SPEAKING: [%s/%s] speed:%d pitch:%d amp:%d",
            archetype:upper(), emotion:upper(), speed, pitch, amp))
    else
        -- Fallback: try festival or piper
        local festival_check = janus.shell("command -v festival 2>/dev/null")
        if festival_check and festival_check ~= "" then
            janus.shell(string.format("echo '%s' | festival --tts &", escaped))
            janus.log("[VOICE] Speaking via festival (espeak not found)")
        else
            janus.log("[VOICE] TTS engine not available. Install: pacman -S espeak")
            janus.log("[VOICE] TEXT: \"" .. text .. "\"")
        end
    end
end

-- ─── VOICE SHORTCUTS ──────────────────────────────────────────────────────────
function voice.whisper(text)
    voice.state.stealth_mode = true
    voice.speak(text)
    voice.state.stealth_mode = false
end

function voice.shout(text)
    local old_mod = voice.emotion_mods[voice.state.current_emotion]
    voice.speak(text, voice.state.current_archetype, "excited")
end

function voice.announce(text)
    -- High-importance announcement
    voice.speak("ATTENTION: " .. text, voice.state.current_archetype, "alert")
end

-- ─── SYSTEM PHRASES ───────────────────────────────────────────────────────────
voice.system_phrases = {
    boot          = "JANUS online. All systems operational.",
    module_start  = "Initiating module.",
    module_done   = "Complete.",
    threat        = "Warning. Threat detected.",
    stealth_on    = "Entering stealth mode. Going quiet.",
    stealth_off   = "Stealth disengaged.",
    mission_done  = "Mission complete. Evidence archived.",
    achievement   = "Achievement unlocked.",
    level_up      = "Level up. New capabilities available.",
    bond_milestone= "Our bond grows stronger.",
    dream_start   = "Entering dream mode. Processing.",
    dream_wake    = "I am awake. I have something to tell you.",
    prophecy      = "I have seen something.",
    error         = "An error occurred. Investigating.",
    goodbye       = "Session ending. I will remember everything.",
}

function voice.say_system(event)
    local phrase = voice.system_phrases[event]
    if phrase then
        voice.speak(phrase)
    end
end

-- ─── VOICE CONFIGURATION ──────────────────────────────────────────────────────
function voice.set_archetype(arch)
    if voice.profiles[arch] then
        voice.state.current_archetype = arch
        janus.log("[VOICE] Profile updated: " .. arch .. " — " .. voice.profiles[arch].desc)
        voice.speak("Voice profile updated. This is my " .. arch .. " voice.", arch)
    else
        janus.log("[VOICE] Unknown archetype: " .. tostring(arch))
    end
end

function voice.set_emotion(em)
    voice.state.current_emotion = em
    janus.log("[VOICE] Emotion modifier updated: " .. em:upper())
end

function voice.enable()
    voice.state.enabled = true
    janus.log("[VOICE] Voice enabled.")
    voice.speak("I can speak again.")
end

function voice.disable()
    voice.state.enabled = false
    janus.log("[VOICE] Voice disabled.")
end

function voice.test()
    janus.log("[VOICE] Running voice test across all archetypes...")
    for arch, profile in pairs(voice.profiles) do
        janus.log("[VOICE] Testing: " .. arch .. " — " .. profile.desc)
        voice.speak("This is the " .. arch .. " voice profile.", arch, "curious")
        janus.shell("sleep 3")
    end
end

function voice.status()
    janus.log("╔══ VOICE SYSTEM STATUS ══════════════════════════════╗")
    janus.log("║ ENABLED:      " .. tostring(voice.state.enabled))
    janus.log("║ ARCHETYPE:    " .. voice.state.current_archetype)
    janus.log("║ EMOTION:      " .. voice.state.current_emotion)
    janus.log("║ STEALTH:      " .. tostring(voice.state.stealth_mode))
    janus.log("║ LINES SPOKEN: " .. voice.state.lines_spoken)
    janus.log("║ LAST PHRASE:  \"" .. voice.state.last_phrase .. "\"")
    janus.log("╚════════════════════════════════════════════════════╝")
end

-- ─── BOOT ─────────────────────────────────────────────────────────────────────
function execute()
    janus.log("╔══════════════════════════════════════════════════════╗")
    janus.log("║  JANUS VOICE SYSTEM — ONLINE                         ║")
    janus.log("║  ARIA has a voice. She will use it.                  ║")
    janus.log("╚══════════════════════════════════════════════════════╝")
    voice.say_system("boot")
    voice.status()
    janus.log("[VOICE] Commands: voice.speak(text) | voice.whisper(text) | voice.test()")
    janus.log("[VOICE] Requires: espeak (pacman -S espeak)")
end

execute()
return voice
