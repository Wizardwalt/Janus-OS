--[{
--  "id": "evidence.read",
--  "name": "Evidence Bounded Read (Read-Only)",
--  "version": "1.0.0",
--  "description": "Read a bounded slice of one customer-approved evidence file as hex. Read-only; never modifies evidence.",
--  "author": "Janus",
--  "priority": 10,
--  "capabilities": ["storage:file_system"],
--  "category": "evidence_readonly",
--  "status": "production"
--}]

-- Read-only bounded read. Requires args.path inside the approved scope.
-- Optional args.offset (default 0) and args.length (default 4096, max 1 MiB).

function execute()
    local path = args and args.path
    if type(path) ~= "string" or path == "" then
        error("args.path must be a non-empty absolute path to an approved evidence file")
    end

    local offset = tonumber(args.offset) or 0
    local length = tonumber(args.length) or 4096
    if offset < 0 then error("args.offset must not be negative") end
    if length <= 0 then error("args.length must be greater than zero") end

    local r = janus.evidence.read(path, offset, length)
    janus.log(string.format("[EVIDENCE] read %d byte(s) at offset %d from %s",
        r.bytes_read, r.offset, r.path))

    return {
        status = "success",
        operation = "read",
        path = r.path,
        offset = r.offset,
        bytes_read = r.bytes_read,
        hex = r.hex,
        read_only = true,
    }
end

return execute()
