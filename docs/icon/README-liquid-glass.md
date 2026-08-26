# Liquid Glass app icon (macOS 26 Tahoe)

On macOS 26, the system renders app icons with **Liquid Glass** — the specular
edge highlight, depth, and Light/Dark/Clear/Tinted variants are applied *live by
the OS* from a **layered** icon authored in **Icon Composer** (`.icon` format).
A flat `.icns`/PNG (what `tauri icon` produces) is shown literally — no glass.

This folder has everything staged to give Agency Agents the native treatment.

## What's here

```
layers/
  Foreground.png         brain-circuit glyph, COLOR, transparent, 1024² (≈71% inset)
  Foreground-white.png   same glyph as a white silhouette (reads great on glass)
  Background.png         deep purple→teal gradient, 1024², full-bleed
  preview-flat.png       flat composite (reference only — NOT the glassed result)
AppIcon.icon/            a starter Icon Composer bundle (icon.json + Assets/)
```

`AppIcon.icon/icon.json` is hand-authored and **validates with `actool`** (zero
schema errors). It uses Background as the base layer and the colored glyph on
top. Swap in `Foreground-white.png` if you prefer the white glyph.

## Authoring (the 2-minute step — Icon Composer is installed)

Standalone `actool` won't compile a Liquid Glass `.icon` (Xcode 26 does it through
an internal build path the CLI doesn't expose), so author/export via the app:

1. Open **Icon Composer** (`/Applications/Xcode.app/Contents/Applications/Icon Composer.app`).
2. Either **open `docs/icon/AppIcon.icon`**, or **New → drag** `Background.png`
   then `Foreground.png` from `layers/` as two layers (background first).
3. Confirm it looks right (it renders the glass live). Tweak glyph scale/position,
   the background fill, and the Dark/Tinted variants if you like.
4. **Export** → choose the **`.icns`** export (Tahoe-compatible, carries the
   layered glass representation).

## Integration into the build — DONE (2026-06-09, this is what's wired)

macOS 26 Tahoe renders app icons from a **compiled `Assets.car`** (Icon
Composer's Liquid Glass), not from `.icns`. Shipping only an `.icns` lands the
app in Tahoe "icon jail" → a blank/gray squircle. `tauri icon` can't produce
`Assets.car` yet (tauri-apps/tauri#14207, #14979), so we compile it ourselves.

**1. Compile `AppIcon.icon` → `Assets.car` (+ a Tahoe-aware `AppIcon.icns`):**
`actool` lives only inside full Xcode (not Command Line Tools), so call it by
path:

```sh
ACTOOL="/Applications/Xcode-beta.app/Contents/Developer/usr/bin/actool"   # or Xcode.app
"$ACTOOL" docs/icon/AppIcon.icon --compile /tmp/iconcar \
  --platform macosx --minimum-deployment-target 15.0 \
  --app-icon AppIcon --include-all-app-icons \
  --output-partial-info-plist /tmp/iconcar/partial.plist
# → /tmp/iconcar/{Assets.car, AppIcon.icns, partial.plist}
cp /tmp/iconcar/Assets.car     src-tauri/Assets.car        # shipped via bundle.resources
cp /tmp/iconcar/AppIcon.icns   src-tauri/icons/icon.icns   # legacy fallback (≤ macOS 25)
```

**2. Wire it (already committed):**
- `tauri.conf.json` → `bundle.resources` includes `"Assets.car"` → lands at
  `Contents/Resources/Assets.car`.
- `src-tauri/Info.plist` (Tauri 2 merges it) → `CFBundleIconName = AppIcon`.
- `bundle.icon` still lists `icons/icon.icns` → Tauri sets `CFBundleIconFile`
  for older macOS. Both paths coexist.

**3. `npm run tauri build`** → Tahoe shows the live glass; older macOS shows the
icns. Verify: `Contents/Resources/` has BOTH `Assets.car` + `icon.icns`, and
Info.plist has BOTH `CFBundleIconName` + `CFBundleIconFile`.

> Re-run step 1 whenever `docs/icon/AppIcon.icon` changes. Do NOT run
> `npm run tauri icon` afterward — it overwrites `icon.icns` with a flat
> (non-glass, Tahoe-jailed) version.

## Regenerating the layers from source art

Source: `brain-circuit.svg` (Affinity export). To rebuild the layers:

```sh
SVG="/path/to/brain-circuit.svg"
rsvg-convert -h 760 "$SVG" -o /tmp/g.png
magick -size 1024x1024 xc:none /tmp/g.png -gravity center -composite layers/Foreground.png
magick /tmp/g.png -channel RGB -evaluate set 100% +channel /tmp/gw.png
magick -size 1024x1024 xc:none /tmp/gw.png -gravity center -composite layers/Foreground-white.png
magick -size 1024x1024 gradient:'#4c1d95'-'#0e7490' layers/Background.png
```
