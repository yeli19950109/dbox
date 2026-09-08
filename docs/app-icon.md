# dbox app icon

The icon combines a mint tool box and a terminal prompt on a dark navy tile,
matching the application palette.

- Cross-platform source: [`src-tauri/app-icon.png`](../src-tauri/app-icon.png)
- Native macOS source: [`AppIcon.icon`](../src-tauri/icons/AppIcon.icon/)
- Full-bleed macOS artwork: [`Artwork.png`](../src-tauri/icons/AppIcon.icon/Assets/Artwork.png)
- Desktop assets: [`src-tauri/icons/`](../src-tauri/icons/)
- Artwork generator: built-in `image_gen` tool. Tauri generates the cross-platform
  formats; Apple's `actool` compiles the native macOS icon.

## macOS icon

The original transparent, inset tile was given an additional gray background by
the newer macOS icon renderer. The native icon uses opaque artwork that extends
to every edge; Apple supplies the rounded-square mask. See Apple's
[app icon guidance](https://developer.apple.com/design/human-interface-guidelines/app-icons).

`src-tauri/Info.plist` selects `AppIcon` with `CFBundleIconName`, and
`tauri.macos.conf.json` bundles the compiled `Assets.car` at the root of
`Contents/Resources`. The compiled `icons/icon.icns` provides the legacy fallback.
Both compiled files are committed, so ordinary builds can use them without
running Icon Composer or recompiling the artwork.

To regenerate the macOS resources on a Mac with Xcode 26 or newer:

```sh
npm run icons:macos
```

If Xcode is installed elsewhere, set `DEVELOPER_DIR` to its `Contents/Developer`
directory. The generation script does not change the system's selected Xcode.

## Cross-platform icons

The existing `bundle.icon` entries in `src-tauri/tauri.conf.json` already point to
the generated desktop assets. To regenerate them from the repository root:

```sh
dbox_icon_output=$(mktemp -d)
npm run tauri -- icon src-tauri/app-icon.png --output "$dbox_icon_output"
cp "$dbox_icon_output"/*.png "$dbox_icon_output"/*.ico "$dbox_icon_output"/*.icns src-tauri/icons/
npm run icons:macos
```

Tauri also generates mobile assets in the temporary directory. Only the desktop
assets are copied into this project. Always regenerate the native macOS icon
last: Tauri's icon command overwrites the compiled legacy ICNS fallback.

## macOS artwork edit prompt

```text
Use case: precise-object-edit
Asset type: full-bleed square artwork for the existing dbox macOS application icon.
Input image: edit target is the existing mint terminal cube on a dark navy rounded-square tile.
Primary request: Keep the mint-green three-dimensional cube, white >_ emblem, their proportions, position, highlights, and all details exactly as in the input. Change ONLY the backdrop: replace the transparent exterior and the rounded-square tile's raised outer border with one continuous opaque dark navy #0c1218 / #121b24 background that fills the entire square image edge to edge, all the way through every corner. Smoothly continue the existing dark navy behind the cube through the entire canvas. Keep the cube's subtle contact shadow.
Composition: Same square canvas and same cube size and placement as input. The artwork will be clipped to Apple's rounded-square mask by the operating system, so the image must NOT contain its own rounded outer tile or inner outline.
Constraints: Absolutely no transparent margin, no baked rounded corners, no inset tile, no visible outer border, no gray, white, or silver framing, no additional objects, no changes to the cube or terminal glyph. All edge and corner pixels must be opaque dark navy. Deliver one square production image only.
```

## Original image generation prompt

```text
Use case: logo-brand
Asset type: production desktop application icon for dbox, a local command-line tool manager built with Tauri.
Primary request: Create one polished, distinctive app icon combining a compact tool/package box with a terminal prompt, reflecting a tool manager called dbox.
Scene/backdrop: A dark navy rounded-square app tile with a truly transparent exterior, on a square 1024x1024 canvas. The tile occupies about 86% of the canvas width, centered with consistent transparent margins. Actual alpha transparency outside the rounded tile, no white backdrop and no baked checkerboard.
Subject: One bold mint-green softly beveled isometric cube/toolbox centered on the tile, with a clean near-white terminal prompt >_ inset on its large front face. The box is solid and closed, a simple cube with three clean planes, no lid flaps, no handles. The terminal glyph should be immediately readable, with a strong chevron and short underscore. The cube fills about 62% of the tile width. Keep the symbol large, strong and optically centered.
Style/medium: Premium macOS desktop app icon with restrained dimensional depth, carefully rounded edges, precise geometry, subtle soft studio highlights and a restrained short shadow. Minimal, calm, crisp, tactile, professionally finished. A single iconic object, not a scene or a mockup.
Color palette: Match the actual dbox UI: dark navy #0c1218 / #121b24 tile, mint #5de1c2 as the dominant box color, deeper teal shaded face, pale mint top face, near-white #e8edf2 terminal glyph. High contrast at small sizes.
Text: Only the terminal glyph >_ as an emblem. No wordmark and no letters.
Constraints: One square icon only. No decorative framing, no extra badges, no fine details, no wires, no floating particles, no neon glow, no magenta, no watermark, no presentation board. Smooth antialiased silhouette and real transparency preserved. Must remain recognizable when reduced to 32x32 pixels.
```
