# M3U fixtures (synthetic)

- `basic.m3u`, `aux_lines.m3u` (EXTVLCOPT / KODIPROP / EXTGRP / pipe headers / credentials in URLs for redaction tests),
  `catchup.m3u` (default/append/shift/xc/flussonic/bad values, Dispatcharr `tv_archive` attrs), `malformed.m3u`
  (BOM, CRLF, orphan lines, unclosed quotes, duplicates, long attribute, odd schemes, no trailing newline).
- `gen-large-m3u.py` (in `tools/fixtures/`) writes a 20k-entry playlist to `m3u/generated/` (gitignored).
- Hosts use `.test`; credentials are synthetic (`testuser` / `testpass`).
