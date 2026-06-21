# Download progress callback (G8)

Status: implemented

## Problem

`Download` drove an indicatif bar straight to the terminal. There was no byte-level
callback, so non-terminal consumers (GUIs, structured logging, tests) could not
observe or render progress, and got terminal control codes they did not want or no
progress at all.

## Decision

A single callback closure: `Download::progress_callback(|downloaded, total| ..)` and
the same `.progress_callback(..)` on every `Update` builder, invoked as the download
streams. `total` is `None` when the server sends no `Content-Length`. It is
independent of indicatif, which stays the zero-config default when
`show_download_progress` is on and no callback is provided. Unknown-length progress
was also fixed (the bar position is pinned within the known size).

See the `progress_callback` setter in `src/lib.rs` / `src/macros.rs` and the
CHANGELOG `[1.0.0]` Added entry.
