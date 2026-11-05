--[{
--  "id": "evidence.hash",
--  "name": "Evidence Hash (Read-Only)",
--  "version": "1.0.0",
--  "description": "Compute SHA-256 of one customer-approved evidence file. Read-only; never modifies evidence.",
--  "author": "Janus",
--  "priority": 10,
--  "capabilities": ["storage:file_system"],
--  "category": "evidence_readonly",
--  "status": "production"
--}]

-- Read-only evidence collection.
-- Requires: args.path (absolute, inside the engagement's approved evidence paths).
-- Performs a real SHA-256 over the real file. No simulation, no writes.

function execute()
    local path = args and args.path
    if type(path) ~= "string" or path == "" then
        error("args.path must be a non-empty absolute path to an approved evidence file")
    end

    local result = janus.evidence.hash(path)
    janus.log(string.format("[EVIDENCE] SHA-256 %s", result.sha256))
    janus.log(string.format("[EVIDENCE] path=%s bytes=%d", result.path, result.size_bytes))

    return {
        status = "success",
        operation = "sha256",
        path = result.path,
        sha256 = result.sha256,
        size_bytes = result.size_bytes,
        read_only = true,
    }
end

return execute()
