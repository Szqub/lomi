# glib 0.18.5 security backport

GTK 0.18 and WebKitGTK 2.0 require the glib 0.18 dependency family. This
directory preserves that API while backporting the upstream fix for
[RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html).

## Source and integrity

- Base: the exact [glib 0.18.5 crates.io archive](https://static.crates.io/crates/glib/glib-0.18.5.crate).
- Archive SHA-256, verified against the official
  [crates.io index](https://index.crates.io/gl/ib/glib):
  `233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`.
- Original upstream source revision, retained in `.cargo_vcs_info.json`:
  `42b9caf98e03ded086362d9653ca58fe94dc8658`.
- Fix: [gtk-rs-core pull request #1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343),
  commit `b5a4071e439bef2b5eea76c3aa25e5ae84839e34`, merged as
  `05dff0ee696f9bcd8617cd48c4b812d046d440cb`.
- Original `src/variant_iter.rs` SHA-256:
  `1fd02859333761c45321b32f28b24233446b97d0022a90d3a937ed162585b90e`.
- Patched `src/variant_iter.rs` SHA-256:
  `a0f5ee8acb8faa089bcdfbc9a57372609fce7654026ccef7d9a224d05a654ccc`.

Every file from the archive is retained. Only two original source lines change:
`VariantStrIter::impl_get` makes the pointer variable mutable and passes
`&mut p` to the variadic C function that writes to it. The applied diff is in
`security-backport.patch`; it matches the upstream fix. This document and that
patch are the only added files. The original MIT `LICENSE` and `COPYRIGHT`
remain unchanged.

## Validation and maintenance

The existing upstream `test_variant_str_iter_nth`,
`test_variant_str_iter_count`, and `test_variant_str_iter_last` tests exercise
forward/reverse iteration and iterator exhaustion. They were copied unchanged
into a temporary harness depending on this directory, then passed with
`cargo test --release --lib` on macOS with native GLib 2.88.3 and the macOS 26.5
SDK. Optimized compilation matters because the original undefined behavior
can fail under optimization. Linux desktop execution remains a separate check.

The root Cargo manifest patches crates.io to this directory, and the desktop
lockfile records glib as a path dependency. `cargo metadata --locked` with the
Linux platform filter confirms that GTK and WebKitGTK resolve the patched crate.
`cargo audit` no longer reports RUSTSEC-2024-0429 for this path dependency; this
scanner result does not inspect or prove the source fix. The archive comparison,
upstream diff, and optimized tests provide the backport evidence. No advisory
ignore is needed or added.

Keep the package version at 0.18.5 to preserve upstream provenance. Replace this
vendor patch when the desktop dependency family supports a maintained glib
release containing the fix. Recheck security advisories when updating it.
