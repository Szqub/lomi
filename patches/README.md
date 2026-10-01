# xterm WebGL rendering fixes

The `@xterm/addon-webgl` 0.19.0 patch keeps one empty canvas/context available
for the next renderer. Creating a WebGL context blocks tab switching for about
200 ms on the tested Linux WebKitGTK setup. Hidden terminals still dispose their
renderers, texture atlases, GPU buffers, textures, programs, shaders, and vertex
arrays; the spare canvas has a zero-sized drawing buffer. Additional spare
contexts are explicitly released. Lost contexts and incompatible document or
context options are not reused.

Terminals with matching appearance share one glyph texture atlas. Clearing it
after another pane's fonts settle also invalidates every owner's cached glyph
coordinates. The patch backports upstream [#6042](https://github.com/xtermjs/xterm.js/pull/6042)
and [#6055](https://github.com/xtermjs/xterm.js/pull/6055): an atlas layout version
changes after clears, page merges, and overflow page creation. Each renderer
tracks its own last-seen version and rebuilds its model once on its next frame.
This prevents fragmented or missing text when another terminal starts without
rebuilding the full viewport on every subsequent frame.

The patch includes the upstream TypeScript sources and both published JavaScript
bundles. Keep them in sync when updating the dependency. `tests/ui/terminal-atlas.spec.ts`
compares painted pixels across sibling startup and shared atlas clearing while
checking that content, selection, geometry, and sessions survive. The resource
regression check is `tests/ui/terminal-gpu.spec.ts`; `terminal-switch.spec.ts`
also checks context loss, background output, selection, and resizing.
`tests/terminal-webgl-package.test.ts` verifies that both patched JavaScript
bundles load, catching incomplete patch output before a release.
