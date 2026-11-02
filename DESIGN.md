# RustCleanup design direction

The anti-slop rules (https://github.com/miqdadbadjuber/anti-slop) are applied as a filter. This file is the
direction they filter. Every decision below has its one-line reason (rule R-31).

**Design Read:** a Windows 11 system utility for people tuning their own PC, in a quiet native-desktop
style, dial **ENERGY 1 / RHYTHM 2 / MOTION 1**.

- ENERGY 1: the tool changes system settings, so it should feel calm and deliberate, not exciting.
- RHYTHM 2: one panel (Memory) is raised because it is the main job; the other sections are plain lists, so the screen is not a grid of identical cards.
- MOTION 1: hover states only. The one exception is the theme fade, which the owner asked for. It stops the whole window from flashing between dark and light, and it is skipped when Windows animations are turned off.

## Identity

| Decision | Reason |
|---|---|
| Accent: oxide `#B4532A`, used only on the Clean RAM button and on switches that are on | Rust is iron oxide; one accent at the main action keeps it meaningful. |
| Neutrals: graphite in dark mode, warm paper in light mode | Warm neutrals sit next to the oxide accent without the cold blue-grey "dev tool" default. |
| Status colours (green, amber, red) only for real states: log levels, memory above 85%, scan results | Colour signals a state, never decoration. |
| Typeface: Segoe UI Variable, falling back to egui's default | It is Windows 11's own UI font, so a tool that changes Windows looks like part of Windows. |
| Monospace: Consolas, only for chkdsk output, process IDs and log times | That output really is console text, and Consolas is the Windows console font. |
| Memory is the only raised panel | It is the screen's focal point and the action most people open the app for. |
| Other sections are headings with hairline-separated rows | Lists read faster than boxed cards for settings, and they make the hierarchy visible. |
| Radius 4 px on controls, 6 px on the Memory panel, pills only on switches | Switches are conventionally pill-shaped; nothing else needs to be. |
| No icons except the theme button (sun and moon) | Text labels describe each action better than generic glyphs. The theme button is icon-only because the owner asked for it, and it has a tooltip and an accessible name. |
| App icon (2.0.1): a white outline of a small brush on pure black, drawn by `assets/make_icon.py` | The owner asked for it. A brush says "cleanup" without text. Black and white stays readable on any taskbar or tray colour. Each size is drawn on its own, so the outline stays at least about 1.3 px wide at 16 px instead of blurring away. It replaces 2.0.0's memory module on an oxide tile. |
| Language picker: flag plus the language's own name | People look for their language under its own name ("Deutsch", not "German"). The flag helps scanning, but the name is what identifies it, since a flag stands for a country, not a language. |
| Themes: follows the Windows app theme on first launch, then remembers your choice | Neither theme is forced (R-21). Both are checked for contrast (R-25, R-34). |

## Copy

- No em dashes, no buzzwords, no emoji.
- Buttons say what they do: "Clean RAM", "Run check", "Open task list", "End task".
- Empty, loading and error states say what happened and what to do next (R-27).
