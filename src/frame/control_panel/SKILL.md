---
name: control-panel-ui-ux
description: "Minimal UI/UX rules for BuckyOS control_panel: pick a style, build a design system, and keep RPC-aligned."
---

# Control Panel UI/UX Skill

Use this when designing or implementing control_panel UI.

## When to Apply

- New UI page or component
- Visual refresh or UX fix
- Aligning UI with RPC data

## Rule Priorities

1. Accessibility: contrast, focus, keyboard nav
2. Interaction: touch targets >= 44x44px, no layout shift on hover
3. Layout: responsive, no horizontal scroll
4. Typography & color: consistent scale and palette
5. Motion: respect prefers-reduced-motion

## Design System (current)

- Primary color: #0f766e
- Accent color: #f59e0b
- Fonts: Space Grotesk (headings), Work Sans (body)

## Workflow

1. Define a design system (style, palette, typography, spacing).
2. Apply UX rules above while building components.
3. Match UI data fields to backend contracts (see `doc/dashboard/README.md`).

## Quick Checks

- No emoji icons; use one SVG icon set.
- 375/768/1024/1440 widths verified.
- Focus states visible.

## References

- `doc/dashboard/README.md`
