# ADR-0003: SQLite is the read model

Status: Accepted (plan decision D5).

## Context

The app serves large lineups (reference: 544 channels; load target 150k) and guides from three kinds of source (Dispatcharr, Xtream,
M3U/XMLTV), each with its own API and identifiers, and must stay usable when a source is slow or offline.

## Decision

Every screen renders from a local SQLite database; each source remains the source of truth and network calls only refresh it.
Internal integer ids with `(source_id, ext_id)` uniqueness (so non-integer M3U and Xtream ids work and multiple sources never
collide), UTC integers for every time, FTS5 external-content search (`channel_fts`, prefix match ranked by number proximity then
`bm25`, under 30 ms on 150k channels), and secrets only in the OS keychain referenced by `secret_ref`.

## Consequences

- One query layer feeds the UI regardless of source kind; the UI branches on `Caps`, never source kind (rule 9).
- Sync code owns reconciliation, staleness and migrations. Filtering (adult, radio, hidden, DRM) lives only in `catalog/`.
- Offline behaviour is defined by the offline matrix (§7.7) rather than by whatever a failed request leaves behind.
- Dwell prefetch is SQLite-only: warming data never touches a stream endpoint.

## What would change it

Sync cost or database size becomes the bottleneck (for example guide windows too large to mirror), or a source offers a reliable
push feed that makes mirroring redundant.

## Implementation notes (2026-11)

- `downlink-core` has the SQLite store with the §7.1 schema, FTS5 and repositories, and a keyring-backed secret store behind a trait.
- Dispatcharr 0.34.1 observations that shaped the model (`docs/dispatcharr-endpoints.md`): channel lists are bare arrays unless
  `page` or `page_size` is sent; the guide grid rows carry only `tvg_id`, so guide-to-channel joins happen on `tvg_id` locally; series
  episode lists and VOD details are lazy on the server (a `provider-info` call populates them), so VOD sync must treat missing detail as
  "not fetched yet"; `effective_stream_profile_id` is null for the server default, so the effective profile is resolved at sync from
  `stream_settings.default_stream_profile`.
- Programme ids are integers for XMLTV-sourced guides locally but must stay string-tolerant (dummy EPG ids).
