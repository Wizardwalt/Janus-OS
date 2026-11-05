--[{
--  "id": "evidence.manifest",
--  "name": "Evidence Chain-of-Custody Manifest (Read-Only)",
--  "version": "1.0.0",
--  "description": "Hash every file in one customer-approved evidence directory and return a chain-of-custody manifest. Read-only.",
--  "author": "Janus",
--  "priority": 10,
--  "capabilities": ["storage:file_system"],
--  "category": "evidence_readonly",
--  "status": "production"
--}]

-- Read-only chain-of-custody manifest for a single approved directory.
--
-- Enumerates the directory, hashes each regular file, and records size and
-- modification time. Symlinks are recorded but never followed, so the manifest
-- cannot be redirected outside the approved scope. Subdirectories are recorded
-- as entries and are not recursed, keeping the operation bounded and auditable.

function execute()
    local path = args and args.path
    if type(path) ~= "string" or path == "" then
        error("args.path must be a non-empty absolute path to an approved evidence directory")
    end

    local entries = janus.evidence.list(path)
    local files, skipped = {}, {}

    for _, entry in ipairs(entries) do
        local full = path
        if string.sub(full, -1) ~= "/" then full = full .. "/" end
        full = full .. entry.name

        if entry.is_symlink then
            skipped[#skipped + 1] = { name = entry.name, reason = "symlink_not_followed" }
        elseif entry.is_dir then
            skipped[#skipped + 1] = { name = entry.name, reason = "directory_not_recursed" }
        elseif entry.is_file then
            local ok, result = pcall(janus.evidence.hash, full)
            if ok then
                files[#files + 1] = {
                    name = entry.name,
                    path = result.path,
                    sha256 = result.sha256,
                    size_bytes = result.size_bytes,
                }
            else
                skipped[#skipped + 1] = { name = entry.name, reason = "unreadable" }
            end
        end
    end

    local total_bytes = 0
    for _, f in ipairs(files) do total_bytes = total_bytes + f.size_bytes end

    janus.log(string.format("[EVIDENCE] manifest: %d file(s) hashed, %d skipped, %d byte(s)",
        #files, #skipped, total_bytes))

    return {
        status = "success",
        operation = "manifest",
        directory = path,
        file_count = #files,
        skipped_count = #skipped,
        total_bytes = total_bytes,
        files = files,
        skipped = skipped,
        read_only = true,
    }
end

return execute()
