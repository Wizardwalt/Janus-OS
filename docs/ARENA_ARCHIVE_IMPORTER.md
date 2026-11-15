# Arena Chat Importer

The Janus Arena Chat Importer is a browser-only intake tool for LM Arena / arena.ai exports.

## Open it

After starting the Janus web server, open:

`http://localhost:5000/arena-importer.html`

From another device on the same trusted network, use the Janus host's address and port 5000. Do not expose the runtime API port 8080 publicly.

## Android / Firefox workflow

1. Export your Arena conversations as JSON files or a ZIP containing JSON files.
2. Save the exports in Downloads.
3. Open JanusOS in Firefox and navigate to `/arena-importer.html`.
4. Tap the file picker, select the ZIP and/or JSON exports, then tap **IMPORT SELECTED FILES**.
5. Review the detected conversation and message counts and any warnings.
6. Tap **DOWNLOAD JANUS ARENA ARCHIVE (.ZIP)**. Check Downloads for `janus-arena-archive.zip`.
7. Keep the original Arena exports as a separate backup as well.

You can select several ZIP/JSON files in one import. If the phone runs low on memory, import smaller batches and keep each file below 150 MB.

## Output layout

- `originals/`: the exact source ZIP/JSON files selected by the user.
- `janus-arena-archive.json`: normalized conversation records. Each record also retains its original raw wrapper and all raw message objects.
- `conversations/`: readable Markdown transcripts.
- `INDEX.md`: chronological index.
- `manifest.json`: import counts, source filenames, duplicate counts, and warnings.

Conversation IDs are used to avoid indexing the same exported conversation more than once. Duplicate source files are still preserved under `originals/`; duplicate locations are recorded on the retained normalized record.

## Privacy and limits

- Source files are read and processed in the browser; conversation content is not posted to the Janus server.
- JSON imports need no third-party library.
- ZIP reading and ZIP archive creation load JSZip 3.10.1 from jsDelivr on demand. An internet connection is needed for ZIP operations unless the library is later bundled locally.
- Per-input-file limit: 150 MB.
- Per-JSON-entry limit: 30 MB.
- Per-ZIP expanded JSON limit: 300 MB.
- Very large imports should be split into batches on mobile.

This first version creates a loss-preserving research archive. It does **not** automatically inject imported text into ARIA's permanent memory or treat historical model suggestions as approved Janus requirements. A later review/import workflow should classify each item as an original idea, user request, approved decision, AI recommendation, implemented, incomplete, or missing before it becomes active project knowledge.
