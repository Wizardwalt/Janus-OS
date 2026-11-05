--[{
--  "id": "evidence.list",
--  "name": "Evidence Directory Listing (Read-Only)",
--  "version": "1.0.0",
--  "description": "List entries of one customer-approved evidence directory. Read-only; does not follow symlinks.",
--  "author": "Janus",
--  "priority": 10,
--  "capabilities": ["storage:file_system"],
--  "category": "evidence_readonly",
--  "status": "production"
--}]

-- Read-only directory enumeration. Requires args.path inside the approved scope.

function execute()
    local path = args and args.path
    if type(path) ~= "string" or path == "" then
        error("args.path must be a non-empty absolute path to an approved evidence directory")
    end

    local entries = janus.evidence.list(path)
    janus.log(string.format("[EVIDENCE] listed %d entries in %s", #entries, path))

    return {
        status = "success",
        operation = "list",
        path = path,
        entry_count = #entries,
        entries = entries,
        read_only = true,
    }
end

return execute()
