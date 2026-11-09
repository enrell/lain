# Patched dependencies

`xattr-0.2.3/`: `xattr` 0.2.3 (MIT/Apache-2.0, unchanged except one line in
`src/sys/mod.rs`). GPUI 0.2.2 reaches it through `gpui_http_client` →
`zed-async-tar` (default feature `xattr`). `lain-client` pins `libc =0.2.191`
(D-172), and libc 0.2.190+ no longer defines `ENOATTR` on Linux, so 0.2.3 does
not compile there. The patch uses `ENODATA`, which is what glibc's `ENOATTR`
is. Drop this patch once GPUI moves to `xattr` 1.x (D-183).
