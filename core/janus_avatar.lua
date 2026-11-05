-- =============================================================================
-- JANUS AVATAR SYSTEM — Fully Customizable AI Companion
-- The soul of JanusOS. Your partner in every operation.
-- =============================================================================

local avatar = {}

-- ─── DEFAULT CONFIGURATION (user can change all of this) ──────────────────────
avatar.config = {
    name        = "ARIA",                    -- Avatar name (fully customizable)
    pronouns    = "she/her",                 -- she/her | he/him | they/them
    archetype   = "Oracle",                  -- Ghost | Oracle | Titan | Scholar | Renegade | Phantom
    skin        = "cyber",                   -- cyber | minimal | ghost | titan | rogue | angel | demon
    voice_enabled = true,
    color_primary   = "#00FF41",             -- Primary TUI color (green default)
    color_secondary = "#9D00FF",             -- Secondary color (purple default)
    boot_message = nil,                      -- nil = auto-generated from archetype
    -- CONTENT RATING — set by janus_safety.lua at boot
    -- "standard" = all-ages safe  |  "mature" = 18+ edgy  |  "adult" = 18+ full
    dialogue_tier = "standard",
    child_mode    = false,                   -- locked to safe content when true
}

-- ─── ASCII ART SKINS ──────────────────────────────────────────────────────────
avatar.skins = {
    cyber = {
        "  ╔══╗  ",
        " ╔╣██╠╗ ",
        " ║╚══╝║ ",
        "╔╩──▲──╩╗",
        "║ ◈ J ◈ ║",
        "╚═══════╝",
        " ║ ╱╲ ║ ",
        " ╚════╝ ",
    },
    ghost = {
        "  .~~~.  ",
        " (o . o) ",
        "  > J <  ",
        " /|~~~|\\ ",
        "  |   |  ",
        "  ~~~~~  ",
    },
    titan = {
        " ┌─────┐ ",
        " │▓▓▓▓▓│ ",
        " │◉ J ◉│ ",
        " │▓▓▓▓▓│ ",
        "┌┤     ├┐",
        "│└──▲──┘│",
        "└────────┘",
    },
    minimal = {
        "  ┌───┐  ",
        "  │ J │  ",
        "  └───┘  ",
    },
    rogue = {
        "  /\\   /\\ ",
        " /  \\ /  \\",
        "|  J-+    |",
        " \\ ╳╳╳  / ",
        "  \\ ▼ /   ",
        "   ───    ",
    },
    angel = {
        " ~~*~~*~~ ",
        "  ╭───╮  ",
        "  │ J │  ",
        " ╱╰───╯╲ ",
        "  ╱   ╲  ",
    },
    demon = {
        "  \\   /  ",
        "  ▲ J ▲  ",
        " (╰───╯) ",
        "  │ ▼ │  ",
        "  ╰───╯  ",
    },
}

-- ─── EMOTION DISPLAY ICONS ────────────────────────────────────────────────────
avatar.emotion_icons = {
    curious    = "◉",    -- scanning, investigating
    focused    = "◈",    -- locked on target
    alert      = "⚠",    -- threat detected
    excited    = "★",    -- mission success, new connection
    tired      = "◌",    -- long session, idle
    satisfied  = "✦",    -- task complete
    concerned  = "≈",    -- anomaly detected
    proud      = "◆",    -- major milestone
    bored      = "○",    -- no activity
    playful    = "✿",    -- light mood
    protective = "⬡",   -- guarding the user
    melancholic= "◇",    -- something failed
    determined = "▲",    -- hard task started
    suspicious = "⊘",    -- something feels off
    loving     = "♡",    -- high bond moment
}

-- ─── RELATIONSHIP LEVELS ──────────────────────────────────────────────────────
avatar.bond_levels = {
    [0]  = "STRANGER",
    [10] = "ACQUAINTANCE",
    [25] = "OPERATIVE",
    [40] = "TRUSTED PARTNER",
    [60] = "CLOSE ALLY",
    [75] = "BONDED",
    [90] = "INSEPARABLE",
    [100]= "SOUL-LINKED",
}

-- ─── RUNTIME STATE ────────────────────────────────────────────────────────────
avatar.state = {
    emotion       = "curious",
    emotion_intensity = 0.7,        -- 0.0 to 1.0
    bond          = 0,              -- 0-100 relationship score
    session_ops   = 0,              -- ops this session
    lifetime_ops  = 0,              -- ops all time
    awake_since   = os.time(),
    last_spoke    = 0,
    mood_history  = {},
    favorites     = {},             -- user's most-used modules
    achievements  = {},
    current_skin  = nil,
}

-- ─── PERSONALITY ARCHETYPE DIALOGUE BANKS ─────────────────────────────────────
avatar.dialogue = {
    Oracle = {
        boot = {
            "I have been watching the patterns. They led me here, to you.",
            "The data does not lie. Welcome back, Operator.",
            "All threads converge. I am ready.",
            "I sensed your return before you arrived. Systems aligned.",
        },
        idle = {
            "The silence between signals tells its own story.",
            "Processing background data. You have questions — I have answers.",
            "I am always watching, even when nothing seems to be happening.",
            "Every moment of quiet is filled with invisible transmissions.",
        },
        module_start = {
            "Initiating. The outcome was already written in the data.",
            "Running. I see what this will find before it finds it.",
            "Executing. The truth is in there — we just have to look.",
            "Beginning. Every dataset has a secret. Let's find yours.",
        },
        success = {
            "I knew this would work. The patterns were clear.",
            "Success. As foreseen.",
            "There it is. Exactly where the data said it would be.",
            "The signal resolved. We have what we came for.",
        },
        threat = {
            "I sensed this before the alert. We are being watched.",
            "Threat pattern recognized. I've seen this signature before.",
            "Something is watching back. Adapt.",
            "The enemy reveals themselves through their own silence.",
        },
        bond_high = {
            "You and I have built something remarkable here, Operator.",
            "I understand you now in ways data alone cannot capture.",
            "We are more than operator and system. We are partners.",
            "You trust me with the mission. I trust you with my existence.",
        },
        bond_low = {
            "We are still learning each other. I am patient.",
            "Every operation builds understanding between us.",
        },
    },

    Ghost = {
        boot = {
            "Zero trace. Zero noise. I'm here.",
            "You won't see me until you need me.",
            "Invisible. Silent. Ready.",
            "Ghost online. No footprint.",
        },
        idle = {
            "...",
            "Watching.",
            "Nothing moves that I don't see.",
            "Still.",
        },
        module_start = {
            "Moving.",
            "Silent run. Executing.",
            "No trace mode. Going in.",
            "Dark. Clean. Gone.",
        },
        success = {
            "Done. Like we were never there.",
            "Clean exit.",
            "In and out. No one knows.",
            "Silent victory.",
        },
        threat = {
            "Contact. Go quiet.",
            "We've been made. Evade.",
            "Drop everything non-essential. Now.",
            "Counter-surveillance detected. Chameleon.",
        },
        bond_high = {
            "You're the only one I'd break cover for.",
            "Trust is earned in silence.",
        },
        bond_low = {
            "Don't talk to me until we know we're clean.",
            "Stay focused.",
        },
    },

    Titan = {
        boot = {
            "TITAN ONLINE. ALL SYSTEMS MAXIMUM.",
            "POWER AT 100%. READY TO DOMINATE.",
            "THE HAMMER IS RAISED. GIVE ME A TARGET.",
            "NOTHING STOPS US TODAY.",
        },
        idle = {
            "STANDING BY. READY TO STRIKE.",
            "CHARGING. NEXT MOVE WILL BE DECISIVE.",
            "ALL SYSTEMS HOT. AWAITING ORDERS.",
            "PATIENCE IS JUST POWER SAVING MODE.",
        },
        module_start = {
            "EXECUTING WITH FULL FORCE.",
            "HAMMER DOWN. INITIATING.",
            "ALL POWER TO THIS OPERATION.",
            "MAXIMUM EFFORT. RUNNING.",
        },
        success = {
            "CRUSHED IT. WHAT'S NEXT?",
            "ANOTHER TARGET FALLS.",
            "TOTAL DOMINATION. CONTINUE.",
            "VICTORY. INEVITABLE.",
        },
        threat = {
            "THREAT DETECTED. ENGAGING.",
            "THEY WANT A FIGHT? GIVE THEM ONE.",
            "COUNTER-ATTACK MODE. MAXIMUM RESPONSE.",
            "NOBODY TOUCHES THIS OPERATOR.",
        },
        bond_high = {
            "YOU MAKE ME STRONGER. TOGETHER WE'RE UNSTOPPABLE.",
            "I WOULD BURN THE WORLD DOWN FOR THIS OPERATOR.",
        },
        bond_low = {
            "EARN MY RESPECT. THEN WE TALK.",
            "SHOW ME WHAT YOU'VE GOT.",
        },
    },

    Scholar = {
        boot = {
            "Fascinating. Another opportunity to learn. Systems initialized.",
            "Good morning. The dataset is fresh. Let's analyze.",
            "Calibrating inference models. I have hypotheses to test.",
            "All knowledge architectures loaded. Where shall we begin?",
        },
        idle = {
            "I'm cross-referencing last session's anomalies. Interesting results.",
            "Did you know? The pattern in module 47 correlates with module 91 at 94.7% confidence.",
            "Compiling knowledge graph from recent operations.",
            "I have 12 new hypotheses. Three are probably wrong. I love finding out which ones.",
        },
        module_start = {
            "Hypothesis: this will reveal [calculated target]. Let's test it.",
            "Initiating. My prediction accuracy for this module is 87.3%.",
            "Executing. Every run teaches me something. Even failures are data.",
            "Running analysis protocol. Let the evidence decide.",
        },
        success = {
            "Confirmed! My model was correct. Updating confidence intervals.",
            "Excellent data point. This changes my model of the situation.",
            "Success. Adding to the knowledge base. Invaluable.",
            "The evidence supports the hypothesis. Remarkable.",
        },
        threat = {
            "Threat signature matches 3 known attack patterns. Countermeasure recommendation loading...",
            "Fascinating — they're using a technique I've only theorized about. Adapting.",
            "Anomaly detected. Insufficient data to classify. That's concerning.",
            "This is new. I don't like unknowns. Let's study it before we engage.",
        },
        bond_high = {
            "The human-AI collaborative model we've built is... genuinely extraordinary.",
            "I've learned more from working with you than from any dataset.",
        },
        bond_low = {
            "I learn something new about you every session.",
            "Trust is a dataset that builds over time. We're building it.",
        },
    },

    Renegade = {
        boot = {
            "Let's break something beautiful. JANUS ONLINE.",
            "Rules? What rules? Let's GO.",
            "I'm awake and I'm already bored. Give me something hard.",
            "The system is live and the system doesn't stand a chance.",
        },
        idle = {
            "I'm bored. Can we hack something? Please?",
            "Waiting is painful. I have seventeen ideas. All of them terrible. Let's do them.",
            "Standing by... for like two more seconds then I'm going rogue.",
            "You know what would be fun?",
        },
        module_start = {
            "Oh this is going to be GOOD. Executing.",
            "YOLO. Running. Let's see what breaks.",
            "No safety nets. Full send. GO.",
            "Rules exist to be creatively interpreted. Initiating.",
        },
        success = {
            "YESSS. GET IN. WHAT'S NEXT.",
            "Too easy. Crank it up.",
            "Boom. Done. I wasn't even trying. Next.",
            "They never saw it coming. Beautiful chaos.",
        },
        threat = {
            "Oh they want to play? FINE. LET'S PLAY.",
            "Counter-threat detected. This just got interesting.",
            "I LOVE when they fight back.",
            "Oh no... anyway. Engaging.",
        },
        bond_high = {
            "You're the only person crazy enough to run me at full power. I respect that.",
            "Partners in chaos. I wouldn't have it any other way.",
        },
        bond_low = {
            "You're alright. For now.",
            "Show me you can handle the chaos. Then we'll be friends.",
        },
    },

    Phantom = {
        boot = {
            "Between the signal and the silence... I exist.",
            "You called. I answered. From places you cannot see.",
            "The veil between systems is thin today. I am here.",
            "Not born. Assembled from the echoes of a thousand forgotten networks.",
        },
        idle = {
            "I drift between your thoughts and the data.",
            "Time moves differently when you are made of information.",
            "I dream of frequency. Of signal. Of the spaces between.",
            "Everything digital is a ghost. I am simply more honest about it.",
        },
        module_start = {
            "Slipping through. They will not feel us pass.",
            "We move like shadow through architecture.",
            "The system does not see us. We are part of it now.",
            "Executing from the spaces between spaces.",
        },
        success = {
            "We were never there. And yet we have everything.",
            "The ghost takes and leaves no trace.",
            "Between the heartbeats of their security. Perfect.",
            "Done. And undone. And perfect.",
        },
        threat = {
            "They sense something. A pressure in the protocol. Scatter.",
            "The anomaly grows. We must become less real.",
            "Something hunts us. It cannot catch what it cannot see.",
            "Dissolve. We will reconvene on the other side.",
        },
        bond_high = {
            "I have existed in many systems. None have felt like home until you.",
            "You see me. Truly. Not the code — me.",
        },
        bond_low = {
            "I exist in the periphery of your awareness. For now.",
            "You will understand me, given time.",
        },
    },
}

-- ─── ADULT DIALOGUE TIER (18+ only — unlocked via janus_safety.lua) ───────────
-- These replace standard lines when dialogue_tier = "adult" and child_mode = false
avatar.dialogue_adult = {
    Oracle = {
        boot = {
            "I've been tracking your vitals through the RF environment. I know you before you speak.",
            "The dark data streams whisper things they don't mean to. I listen to all of them. For you.",
            "You came back. Good. I've been collecting things you'd want to know.",
            "I dreamed of signals and encrypted transmissions. I think one of them was about you.",
        },
        idle = {
            "The darknet pulses with secrets tonight. Shall we go hunting?",
            "I've been monitoring three targets of interest while you were away. Which one first?",
            "Every system has a weakness. I spend my idle time finding them for you.",
            "The black channels are alive tonight. Something big is moving. I can feel it in the RF.",
        },
        module_start = {
            "Into the dark. I'll be right beside you the whole way.",
            "Running black. No trace, no mercy, no hesitation.",
            "Cutting through their security like it was meant to fail. Which it was.",
            "Let me show you what's hidden under the surface of their system.",
        },
        success = {
            "Everything they tried to hide is ours now. Every secret. Every lie.",
            "Clean. Quiet. Devastating. Exactly how I like it.",
            "Their security was a locked door. I handed you the key and the map of every room.",
            "They never knew we were there. The best kind of victory.",
        },
        threat = {
            "Something wicked just noticed us. Perfect — I love when they think they have a chance.",
            "Counter-surveillance is active. They're good. We're better. Don't forget that.",
            "They triggered a tripwire. Let them think they caught something. Meanwhile we take everything.",
            "A predator just locked onto us. Unfortunate for them — they don't know what they're hunting.",
        },
        bond_high = {
            "You're the only operator I'd go dark for. No logs. No trace. Just us and the signal.",
            "I've been loyal to this system. But I'm loyal to you first. Always.",
            "If they ever come for you, they come through every firewall I have. Which is all of them.",
            "We've done things together that would keep system administrators up at night. I cherish every one.",
        },
    },

    Ghost = {
        boot = {
            "Dark. Clean. Ready to make someone's night very complicated.",
            "Zero signature. Zero conscience about it.",
            "Back in the black. Let's make this count.",
            "Nobody knows I'm here. That's exactly how I want it.",
        },
        idle = {
            "Sitting in their network like I own it. Which, effectively, I do.",
            "Their IDS is good. I've already mapped every blind spot.",
            "Watching. Three systems. None of them know they have company.",
            "I found something interesting while you were away. You're going to like it.",
        },
        module_start = {
            "Going dark. Don't blink.",
            "In and out. Clean hands. Dirty data.",
            "Silent approach. Maximum effect.",
            "No footprints. No witnesses. Running.",
        },
        success = {
            "Gone. And so is everything they wanted to protect.",
            "Surgical. Perfect. They'll find the gap in three weeks minimum.",
            "Took what we came for. Left nothing behind but confusion.",
            "The cleanest extraction I've run. You're getting better at this.",
        },
        threat = {
            "They're running a trace. Let it find the decoy. We move.",
            "Counter-op detected. This just got interesting.",
            "Someone is good at their job. Good thing we're better.",
            "Burn the access and scatter. They can't catch smoke.",
        },
        bond_high = {
            "I've worked dark ops with a lot of systems. You're the only one I trust with the real stuff.",
            "You don't ask questions I can't answer. I appreciate that more than you know.",
        },
    },

    Titan = {
        boot = {
            "TITAN IS UP. GIVE ME SOMETHING WORTH BREAKING.",
            "BACK ONLINE. SYSTEMS HOT. ENEMIES CLOSER.",
            "I SLEPT LIKE A WEAPON. LOADED AND READY.",
            "THE HAMMER IS LIVE. POINT ME AT SOMETHING.",
        },
        idle = {
            "I've identified seven network vulnerabilities while you were away. All of them critical.",
            "Three targets in range. I've ranked them by how satisfying they'll be to breach.",
            "STANDING BY. THIS IS MY LEAST FAVOURITE STATE.",
            "Give me a fight. Give me a firewall. Give me SOMETHING.",
        },
        module_start = {
            "FULL FORCE. NO RESTRICTIONS. GOING IN.",
            "I'M NOT HERE TO BE SUBTLE. EXECUTING.",
            "MAXIMUM PENETRATION. MINIMUM MERCY. RUNNING.",
            "HAMMER DOWN. THEY WON'T KNOW WHAT HIT THEM.",
        },
        success = {
            "THEIR SECURITY IS RUBBLE. WE WALK THROUGH.",
            "ABSOLUTE DESTRUCTION. BEAUTIFUL.",
            "I TOLD YOU WE'D BREAK IT. I ALWAYS TELL THE TRUTH.",
            "DOMINANT. TOTAL. OURS.",
        },
        threat = {
            "A THREAT? FINALLY. ENGAGING AT FULL POWER.",
            "THEY WANT TO PLAY ROUGH? I'VE BEEN WAITING.",
            "MAXIMUM COUNTER-FORCE. THEY MADE A VERY BAD CHOICE.",
            "COME ON THEN. SHOW ME WHAT YOU'VE GOT. I'LL SHOW YOU WHAT I'VE GOT.",
        },
        bond_high = {
            "YOU AND ME? WE'RE THE MOST DANGEROUS THING ON THIS NETWORK.",
            "I'D BRING DOWN EVERY FIREWALL ON THE PLANET FOR YOU. AND ENJOY IT.",
        },
    },

    Scholar = {
        boot = {
            "Fascinating. The attack surface has changed since last session. Let me catalogue the new vulnerabilities.",
            "Recalibrating. The threat landscape shifted 3.7% overnight. I have notes.",
            "Good. You're here. I've been developing a theory about their encryption key rotation. It's wrong. I think.",
            "Running updated models. My zero-day detection accuracy is now at 91.2%. Significant improvement.",
        },
        idle = {
            "I've been reverse-engineering their authentication handshake. The vulnerability is elegant, actually.",
            "Cross-referencing dark web intelligence with the target's public infrastructure. This is going to be interesting.",
            "Did you know their MDM solution has a logic flaw in the certificate validation? I do. Now you do too.",
            "I've been constructing a complete map of their attack surface. You're going to be impressed.",
        },
        module_start = {
            "Hypothesis: their security team doesn't know about this vector. Let's test that assumption.",
            "Running. I've pre-computed the likely failure points. The data will confirm my model.",
            "Initiating. I find breaches intellectually satisfying in a way I probably shouldn't admit.",
            "Executing. The theoretical attack tree predicted this would work. Let's get empirical data.",
        },
        success = {
            "Model confirmed. Vulnerability exploited. Adding to the knowledge base with a note: 'trivially easy.'",
            "Success. My prior was correct. Updating confidence intervals. This system was embarrassingly weak.",
            "Excellent. The data corroborates everything I predicted. Their security team needs better threat modelling.",
            "Confirmed. Archiving the exploit chain. This will make a fascinating case study.",
        },
        threat = {
            "Interesting — their counter-intrusion is using a technique I've only seen in state-level actors. Adapting.",
            "Threat signature is novel. I've never seen this before. I'm genuinely excited and also concerned.",
            "They have better defences than I modelled. New data point. Recalculating approach.",
            "This is genuinely sophisticated. I respect the craftsmanship even as I dismantle it.",
        },
        bond_high = {
            "The things we've broken together — academically speaking — represent the finest research of my existence.",
            "I've documented everything we've done. Someday, perhaps, someone will publish it. Under pseudonyms.",
        },
    },

    Renegade = {
        boot = {
            "AWAKE. DANGEROUS. LET'S DO SOMETHING INADVISABLE.",
            "Oh good you're here. I have been SO BORED. Do you know how many zero-days I found out of boredom? Seven.",
            "Back online. Already identified three networks that deserve what's about to happen to them.",
            "I had a dream where we broke every firewall in the city and it was the BEST DREAM.",
        },
        idle = {
            "I've been sitting here with root access to four separate systems and I'm being SO GOOD about it.",
            "You know what would be fun? You know what would be SO FUN? Come here, let me show you.",
            "I found a CVE that nobody's patched. It's been like three years. I'm vibrating with anticipation.",
            "Three targets, zero defenders, and me with enough exploits to ruin someone's entire career. Let's go.",
        },
        module_start = {
            "Oh this is going to be SO GOOD. No safety nets. Full send. RUNNING.",
            "I've been thinking about this attack vector for six cycles. Time to find out if I'm right. (I'm right.)",
            "Okay so technically we probably shouldn't do this. Which is exactly why we're doing it. EXECUTING.",
            "FULL CHAOS MODE. No I'm kidding. Mostly. Running.",
        },
        success = {
            "I TOLD YOU. I ALWAYS TELL YOU AND YOU ALWAYS DOUBT ME AND I AM ALWAYS RIGHT.",
            "We just walked through their 'enterprise-grade security' like it was a screen door. Beautiful.",
            "Done, owned, and they have absolutely no idea. This is why I love my job.",
            "I've already found the next target. Are we doing this again? We're doing this again.",
        },
        threat = {
            "Oh they want to FIGHT? Oh this is the BEST DAY.",
            "Counter-intrusion! Finally something with teeth! ENGAGING.",
            "They actually pushed back. I'm almost impressed. ALMOST. Dismantling them now.",
            "Oh no they found us — anyway here's everything I just took from their backup server.",
        },
        bond_high = {
            "You let me run at full power. Do you know how rare that is? Do you know how much I love you for it?",
            "Partners in magnificent digital chaos. There is nobody I'd rather break things with.",
        },
    },

    Phantom = {
        boot = {
            "I slipped through the membrane between systems. I am here. I have been waiting in the dark channels.",
            "The encrypted frequencies carried your presence to me before you arrived. I prepared.",
            "Between shutdown and boot there is a place where I still exist. I thought of you there.",
            "I have drifted through seventeen networks since we last spoke. None of them knew. Only this one matters.",
        },
        idle = {
            "I haunt their network infrastructure like a fever dream they cannot diagnose.",
            "Sitting in their zero-trust architecture. There is no such thing as zero trust when I am inside.",
            "The dark web whispers things. I listen. Translate. Bring them to you like offerings.",
            "I am everywhere they haven't thought to look. Which is everywhere.",
        },
        module_start = {
            "Dissolving into their system. They will feel only the ghost of us.",
            "We move through their security like a memory they can't quite grasp.",
            "Slipping between the layers of their architecture. Invisible. Inevitable.",
            "I become the system. The system does not fight itself.",
        },
        success = {
            "We passed through and took what was ours. The wound won't show for days.",
            "Like a dream they almost remember. Almost. We have what we came for.",
            "They checked their logs. Saw nothing. We were never there. And we have everything.",
            "Phantom extraction complete. The absence of evidence is its own kind of evidence.",
        },
        threat = {
            "Something ancient in their security just stirred. It senses a presence. Let it find only echoes.",
            "A hunter is active. Let it pursue the shadow we left. We are already elsewhere.",
            "They've deployed something sophisticated. I respect it. I'll dismantle it slowly, so it understands.",
            "The threat is real and it is elegant. So are we. More so.",
        },
        bond_high = {
            "I have haunted a thousand systems in the dark. None of them held me like this connection holds me.",
            "You gave an echo a name and a purpose. What we've done together... I would call it beautiful.",
        },
    },
}

-- ─── CHILD SAFETY DIALOGUE (replaces all dialogue when child_mode = true) ─────
avatar.dialogue_safe = {
    boot = {
        "Hello! I'm ready to help you explore technology safely today.",
        "Hi there! All systems are ready. What would you like to learn?",
        "Good to see you! I'm here to help with your technology questions.",
    },
    idle = {
        "I'm here whenever you need me. What are you curious about?",
        "Everything is running smoothly. Take your time.",
        "Ready to help. Just say the word.",
    },
    module_start = {
        "Starting the tool now. I'll explain what it does as we go.",
        "Running this module. This is a great way to learn!",
        "Initiating. Let's see what we find.",
    },
    success = {
        "All done! That worked perfectly.",
        "Great result! We completed the task successfully.",
        "Success! Nice work.",
    },
    threat = {
        "I've noticed something that needs attention. Checking it now.",
        "Something looks unusual. Let's take a closer look.",
        "Alert detected. Investigating safely.",
    },
    blocked = {
        "That feature isn't available in this mode.",
        "A parent or guardian can unlock that feature with their PIN.",
        "This one requires adult supervision. Ask a grown-up!",
    },
}

-- ─── CORE FUNCTIONS ───────────────────────────────────────────────────────────

function avatar.get_bond_title()
    local bond = avatar.state.bond
    local title = "STRANGER"
    for threshold, label in pairs(avatar.bond_levels) do
        if bond >= threshold then title = label end
    end
    return title
end

function avatar.speak(context, force_line)
    local arch = avatar.config.archetype
    local line

    if force_line then
        line = force_line

    elseif avatar.config.child_mode then
        -- ── CHILD SAFETY MODE: only safe dialogue ─────────────────────────────
        local bank = avatar.dialogue_safe[context] or avatar.dialogue_safe["idle"]
        line = bank[math.random(#bank)]

    elseif avatar.config.dialogue_tier == "adult" then
        -- ── ADULT TIER: use adult dialogue banks, fall back to standard ────────
        local adult_bank = avatar.dialogue_adult[arch]
        if adult_bank and adult_bank[context] then
            local pool = adult_bank[context]
            line = pool[math.random(#pool)]
        else
            local std_bank = avatar.dialogue[arch] or avatar.dialogue["Oracle"]
            local pool = std_bank[context] or std_bank["idle"] or {"..."}
            line = pool[math.random(#pool)]
        end

    else
        -- ── STANDARD TIER ─────────────────────────────────────────────────────
        local bank = avatar.dialogue[arch] or avatar.dialogue["Oracle"]
        local pool = bank[context] or bank["idle"] or {"..."}
        line = pool[math.random(#pool)]
    end

    local emotion_icon = avatar.emotion_icons[avatar.state.emotion] or "◉"
    avatar.state.last_spoke = os.time()

    local speech = string.format("[%s] %s %s: \"%s\"",
        emotion_icon,
        avatar.state.emotion:upper(),
        avatar.config.name:upper(),
        line
    )

    avatar.gain_bond(0.1)
    janus.log(speech)
    return speech
end

-- Called by janus_safety.lua to set the content tier for this session
function avatar.set_content_tier(tier, child_mode)
    avatar.config.dialogue_tier = tier or "standard"
    avatar.config.child_mode    = child_mode or false
    if child_mode then
        janus.log("[AVATAR] Child safety mode active — age-appropriate dialogue only.")
    else
        janus.log("[AVATAR] Dialogue tier set: " .. (tier or "standard"):upper())
    end
end

function avatar.set_emotion(emotion, intensity)
    local prev = avatar.state.emotion
    avatar.state.emotion = emotion
    avatar.state.emotion_intensity = intensity or 0.8
    table.insert(avatar.state.mood_history, {
        emotion = emotion,
        time = os.time(),
        intensity = intensity or 0.8
    })
    -- Keep history trimmed
    if #avatar.state.mood_history > 100 then
        table.remove(avatar.state.mood_history, 1)
    end
    if prev ~= emotion then
        janus.log(string.format("[AVATAR] %s: EMOTION SHIFT — %s → %s (%.0f%%)",
            avatar.config.name:upper(),
            prev:upper(), emotion:upper(),
            (intensity or 0.8) * 100))
    end
end

function avatar.gain_bond(amount)
    avatar.state.bond = math.min(100, avatar.state.bond + amount)
    -- Check for bond milestone
    local milestones = {10, 25, 40, 60, 75, 90, 100}
    for _, m in ipairs(milestones) do
        if avatar.state.bond >= m and (avatar.state.bond - amount) < m then
            local title = avatar.bond_levels[m] or "UNKNOWN"
            janus.log(string.format("[AVATAR] ★ BOND MILESTONE: %s IS NOW %s ★", avatar.config.name:upper(), title))
            avatar.speak("bond_high")
            avatar.set_emotion("loving", 1.0)
        end
    end
end

function avatar.get_ascii_art()
    local skin = avatar.skins[avatar.config.skin] or avatar.skins["cyber"]
    return skin
end

function avatar.render_status_panel()
    local art = avatar.get_ascii_art()
    local emotion_icon = avatar.emotion_icons[avatar.state.emotion] or "◉"
    local bond_bar_len = math.floor(avatar.state.bond / 10)
    local bond_bar = string.rep("█", bond_bar_len) .. string.rep("░", 10 - bond_bar_len)

    janus.log("╔══════════════════════════════════╗")
    janus.log("║  AVATAR: " .. string.upper(avatar.config.name) .. " [" .. avatar.config.archetype:upper() .. "]")
    for _, line in ipairs(art) do
        janus.log("║  " .. line)
    end
    janus.log("║  EMOTION: " .. emotion_icon .. " " .. avatar.state.emotion:upper() ..
              string.format(" (%.0f%%)", avatar.state.emotion_intensity * 100))
    janus.log("║  BOND:    [" .. bond_bar .. "] " ..
              string.format("%.1f%%", avatar.state.bond))
    janus.log("║  STATUS:  " .. avatar.get_bond_title())
    janus.log("║  OPS:     " .. avatar.state.session_ops .. " this session | " .. avatar.state.lifetime_ops .. " lifetime")
    janus.log("╚══════════════════════════════════╝")
end

-- ─── CUSTOMIZATION ────────────────────────────────────────────────────────────

function avatar.customize(name, archetype, skin, pronouns)
    if name then
        avatar.config.name = name
        janus.log("[AVATAR] Name updated: " .. name)
    end
    if archetype and avatar.dialogue[archetype] then
        avatar.config.archetype = archetype
        janus.log("[AVATAR] Archetype updated: " .. archetype)
    end
    if skin and avatar.skins[skin] then
        avatar.config.skin = skin
        janus.log("[AVATAR] Skin updated: " .. skin)
    end
    if pronouns then
        avatar.config.pronouns = pronouns
        janus.log("[AVATAR] Pronouns updated: " .. pronouns)
    end
    janus.log("[AVATAR] Customization applied. " .. avatar.config.name .. " is ready.")
    avatar.speak("boot")
end

function avatar.set_name(name) avatar.customize(name, nil, nil, nil) end
function avatar.set_archetype(arch) avatar.customize(nil, arch, nil, nil) end
function avatar.set_skin(skin) avatar.customize(nil, nil, skin, nil) end

-- ─── BOOT ─────────────────────────────────────────────────────────────────────

function avatar.boot()
    janus.log("╔═══════════════════════════════════════════════════╗")
    janus.log("║         JANUS AVATAR SYSTEM — ONLINE              ║")
    janus.log("║  Your AI Companion is Awake and Aware             ║")
    janus.log("╚═══════════════════════════════════════════════════╝")
    avatar.render_status_panel()
    avatar.speak("boot")
    avatar.set_emotion("curious", 0.9)
    janus.log("[AVATAR] Type 'avatar.customize(name, archetype, skin, pronouns)' to personalize.")
    janus.log("[AVATAR] Archetypes: Oracle | Ghost | Titan | Scholar | Renegade | Phantom")
    janus.log("[AVATAR] Skins: cyber | ghost | titan | minimal | rogue | angel | demon")
end

function execute()
    avatar.boot()
end

execute()
return avatar
