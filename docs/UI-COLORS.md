# Klyne UI colours

All colours are CSS variables in `apps/studio/web/production.css` (block "UI colour system"). Orange (`--brand` #ff8a3d / `--st-acting`) stays the brand and the "acting" colour. Every state pairs a colour with a text label and an icon, so meaning never depends on colour alone. The state is set as `data-ui` on `<html>`, the header status pill, sidebar chips and the inline card.

## State roles

| Role | Variable | Hex | Label | Icon | Contrast on #121214 | Contrast on chip bg (10% tint) | Dark text #151515 on it (buttons) |
|---|---|---|---|---|---|---|---|
| idle | `--st-idle` | #a3a7ae | Ready | ○ | 7.75 | 6.67 | 7.56 |
| thinking | `--st-thinking` | #7cb4ff | Thinking | ◌ | 8.76 | 7.46 | 8.55 |
| acting | `--st-acting` | #ff9a52 | Working | ▶ | 8.91 | 7.60 | 8.69 |
| approval | `--st-approval` | #ffd166 | Needs your approval | ⚠ | 12.98 | 10.55 | 12.66 |
| input | `--st-input` | #c4a1ff | Needs your answer | ? | 8.81 | 7.49 | 8.60 |
| done | `--st-done` | #5ad79a | Done | ✓ | 10.35 | 8.69 | 10.11 |
| blocked | `--st-blocked` | #ff8fa3 | Couldn’t finish | ⏸ | 8.65 | 7.40 | 8.44 |
| error | `--st-error` | #ff6b6b | Error | ✕ | 6.74 | 5.96 | 6.58 |
| off | `--st-off` | #8b9098 | Access off | 🔒 + dashed border | 5.83 | 5.15 | 5.69 |

Where it shows: header status pill, sidebar chip, orb tint + ring, status line under the orb, inline callout card (approval / answer / couldn’t finish / error / done). Only **needs approval** pulses (2.4 s, 4 px glow); with `prefers-reduced-motion: reduce` it becomes a static outline and all other motion is off.

## Capability identity hues

| Capability | Variable | Hex | Contrast on #121214 |
|---|---|---|---|
| files | `--cap-files` | #4fd1c5 | 10.03 |
| terminal | `--cap-terminal` | #6ee7a0 | 12.12 |
| browser | `--cap-browser` | #7cb4ff | 8.76 |
| desktop | `--cap-desktop` | #5ee0f0 | 11.94 |
| apps | `--cap-apps` | #b79cff | 8.21 |
| skills | `--cap-skills` | #f6d55c | 13.00 |
| memory | `--cap-memory` | #f9a8d4 | 10.32 |
| runtime | `--cap-runtime` | #a9b4c4 | 8.92 |

A capability card uses its hue for the icon, a faint border when available, and a full border + glow while in use. **Access off** is a grey dashed border with a 🔒 “Access off” label (text #a9adb3, 8.30:1), never just faded opacity.

## Rules

- Text minimum 12px (11px only for monospace metadata such as chips and step numbers).
- All state/capability colours above pass WCAG AA (4.5:1) as text on the dark surfaces they are used on.
- Do not add new colours ad hoc: add a role or capability variable here first.
