# Mewtual cat art

Line-art derivatives of the brand cat in `assets/mewtual-logo.svg`: the same chubby wide head, outward-leaning ears (near-vertical outer edge, 0.7-slope inner edge), forehead dip, tiny triangle nose and little omega mouth, redrawn as monochrome icons. Everything inherits colour (`stroke="currentColor"` for lines, `fill="currentColor"` for eyes/nose) so the art follows whatever palette the app theme sets; strokes are 1.6 with round caps/joins to match the existing icon family, and no detail is finer than ~0.8 viewBox units so it stays legible at 14 to 16 px.

## Files

- `icon-cat.svg` (viewBox 0 0 24 24): replacement for the `icoCat` snippet; the "add reaction" / emoji-picker button. Happy closed eyes and whiskers, straight from the logo cat.
- `mascot-idle.svg` (viewBox 0 0 20 20): titlebar mascot, awake and content.
- `mascot-blink.svg` (viewBox 0 0 20 20): identical to idle with eyes closed; show for one frame (~120 ms) every ~30 s.
- `mascot-sleep.svg` (viewBox 0 0 20 20): deep happy-closed eyes plus a tiny "z"; app is idle.
- `mascot-alert.svg` (viewBox 0 0 20 20): wide eyes and two surprise ticks above the forehead; you were mentioned.
- `mascot-sync.svg` (viewBox 0 0 20 20): eyes and muzzle shifted to one side, tail raised; data is moving.
- `ears.svg` (viewBox 0 0 40 12): two solid ear silhouettes only, rising from the bottom edge of the viewBox. Sit it flush along the top edge of the titlebar so the window itself grows subtle cat ears; ear angles match the logo cat.

## Swapping mascot frames

All five mascots share an identical head path, ear angles and face layout; only eyes, muzzle position and accessory marks (z, ticks, tail) differ. Swap the whole SVG (or toggle visibility between pre-rendered copies) and the change reads as an expression shift rather than a morph: idle <-> blink for the periodic blink, idle -> alert on mention, idle -> sync while transferring, idle -> sleep after inactivity.
