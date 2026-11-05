-- src/lua/ai_manager_enhanced.lua
-- Enhanced Overseer AI with tiered offline/online mode and voice synthesis

local ai_manager = {}
ai_manager.online = false
ai_manager.model = "mistral"
ai_manager.voice_enabled = true

-- Initialize network checker
local function check_network()
    -- Simulated network check
    -- In production, use actual network interface checks
    return os.time() % 2 == 0  -- Simplified check
end

-- Overseer AI personality
local overseer_personality = {
    name = "Aria",
    vault_id = "VAULT-OMEGA",
    responses = {
        greeting = "Welcome back, Wanderer. I am Aria, your Overseer. What brings you to the terminal?",
        offline = "I'm operating in offline mode. My responses are limited to local knowledge base.",
        online = "Network connection established. Full AI capabilities available.",
        battery_warning = "Battery level critical. Entering ultra-low power mode.",
        threat_detected = "ALERT: Anomalous signal detected. Threat level elevated.",
    }
}

-- Local regex patterns for offline mode
local offline_patterns = {
    battery = "Battery status: Checking power levels. Consider enabling low%-power mode for extended operation.",
    status = "System status: All core modules operational. No threats detected.",
    help = "Available commands: battery, status, help, network, time. Use 'network' to check connectivity.",
    network = "Network status: Currently offline. Mesh network available. Waiting for connection...",
    time = function() return string.format("Current time: %s", os.date("%Y-%m-%d %H:%M:%S")) end,
    threat = "No immediate threats detected. Environmental sensors operational.",
    hello = overseer_personality.responses.greeting,
}

-- Initialize AI manager
function ai_manager.init()
    ai_manager.online = check_network()
    if ai_manager.online then
        print(overseer_personality.responses.online)
    else
        print(overseer_personality.responses.offline)
    end
    return true
end

-- Respond to input
function ai_manager.respond(input)
    local input_lower = string.lower(input)
    
    if not ai_manager.online then
        return ai_manager.offline_response(input_lower)
    else
        return ai_manager.online_response(input)
    end
end

-- Offline response using regex patterns
function ai_manager.offline_response(input_lower)
    for keyword, response in pairs(offline_patterns) do
        if string.find(input_lower, keyword) then
            if type(response) == "function" then
                return response()
            else
                return response
            end
        end
    end
    return "I'm currently in offline mode. For advanced queries, please establish a network connection."
end

-- Online response (would call Ollama/LLM)
function ai_manager.online_response(input)
    -- In production, this would call the Rust Ollama bridge
    return string.format("[LLM Response] Processing: %s", input)
end

-- Synthesize voice output
function ai_manager.speak(text)
    if ai_manager.voice_enabled then
        -- Would call system TTS here
        print("[Voice Output]: " .. text)
        return true
    end
    return false
end

-- Battery-efficient mode toggle
function ai_manager.set_low_power_mode(enabled)
    if enabled then
        print("Low power mode ENABLED. AI limited to essential responses.")
        ai_manager.model = "nano"  -- Use smaller model
    else
        print("Low power mode DISABLED. Full AI capabilities restored.")
        ai_manager.model = "mistral"
    end
end

-- Check network periodically
function ai_manager.periodic_check()
    local was_online = ai_manager.online
    ai_manager.online = check_network()
    
    if was_online and not ai_manager.online then
        print("[WARNING] Network connection lost. Switching to offline mode.")
    elseif not was_online and ai_manager.online then
        print("[INFO] Network restored. Online mode activated.")
    end
end

return ai_manager
