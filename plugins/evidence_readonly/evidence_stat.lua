--[{
--  "id": "evidence.stat",
--  "name": "Evidence Stat (Read-Only)",
--  "version": "1.0.0",
--  "description": "Report real filesystem metadata for one customer-approved evidence path. Read-only.",
--  "author": "Janus",
--  "priority": 10,
--  "capabilities": ["storage:file_system"],
--  "category": "evidence_readonly",
--  "status": "production"
--}]

-- Read-only evidence metadata. Requires args.path inside the approved scope.

function execute()
    local path = args and args.path
    if type(path) ~= "string" or path == "" then
        error("args.path must be a non-empty absolute path to an approved evidence path")
    end

    local st = janus.evidence.stat(path)
    janus.log(string.format("[EVIDENCE] stat %s (%d bytes)", st.path, st.size_bytes))

    return {
        status = "success",
        operation = "stat",
        path = st.path,
        size_bytes = st.size_bytes,
        is_file = st.is_file,
        is_dir = st.is_dir,
        readonly = st.readonly,
        modified_unix = st.modified_unix,
        read_only = true,
    }
end

return execute()
