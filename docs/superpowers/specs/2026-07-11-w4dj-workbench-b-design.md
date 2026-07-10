# W4DJ Workbench B Design

Date: 2026-07-11

## Summary

This design selects option B, a clean glass workbench layout for the W4DJ desktop frontend. The current dark, hardware-like single-panel presentation is replaced with a calmer productivity layout: a global left rail, two stacked sync channels, and a compact control strip that keeps the mode and format controls visible without fighting the task cards.

## Goals

- Make the app feel more legible on first launch.
- Keep the two sync channels equally visible and easy to compare.
- Keep the language switch in the top-right corner.
- Keep the global mode and lossless format controls visible, but less dominant than the sync slots.
- Preserve the current sync behavior, i18n behavior, and fallback logic.
- Improve spacing, hierarchy, and copy density so the app reads cleanly on Windows and macOS.

## Non-Goals

- No change to sync engine behavior.
- No change to slot count.
- No change to the source/destination fallback rule.
- No advanced settings page.
- No per-platform layout fork.

## Current Problem

The current interface is too close to a machine panel. It uses:

- strong dark contrast everywhere
- dense borders and repeated accent lines
- horizontal layout pressure on smaller windows
- control blocks that compete with the two sync slots

That works for a technical utility, but it is not the clearest presentation for manual demos or first-time users.

## Chosen Direction

B is a workbench layout, not a dashboard.

The core idea:

- a narrow global sidebar on the left
- two large sync cards stacked vertically on the right
- a lighter glass surface language
- fewer decorative lines
- stronger whitespace and clearer labels

This keeps the app readable when the window is medium-sized and makes the primary flow obvious:

1. pick source
2. pick output
3. choose mode
4. start sync

## Information Hierarchy

From highest to lowest priority:

1. App identity and language switch
2. Two sync channels
3. Global mode and lossless format
4. Status, progress, and current file
5. Logs and troubleshooting detail

The new layout should make the global controls feel like global controls, not like a third competing card.

## Layout

### Desktop Layout

The desktop window is split into two regions:

- Left rail: app identity, language switch, global mode, lossless format, and short guidance copy
- Main area: two sync channel cards stacked vertically

### Left Rail

The left rail should stay visually lighter than the sync cards.

Contents:

- eyebrow / product label
- title
- EN / 中文 toggle at the top-right of the window
- mode selector
- lossless format selector shown only in lossless mode
- one or two short lines explaining the selected mode

### Main Area

The main area contains two channel cards.

Each card contains:

- channel number and status badge
- source directory picker
- destination directory picker
- primary action button
- compact progress strip
- expandable log drawer

## Component Rules

### Sync Channel Card

Each channel card should be visually equal and independent.

Rules:

- use a light glass surface with subtle blur or softened translucency
- keep the accent color per channel, but make it quieter than the current theme
- keep path controls wide and readable
- keep the primary action button large enough for touch and pointer use
- keep logs collapsed by default

### Global Control Rail

The global rail should not look like a second set of path pickers.

Rules:

- mode buttons should be grouped together
- lossless format buttons should appear only when lossless mode is active
- explanatory copy should sit below the controls, not inline with the card title
- the rail should be compact enough that it does not steal attention from the sync slots

### Language Switch

The language switch remains in the top-right corner.

Rules:

- keep it separate from the main content flow
- keep it small and always visible
- show `EN` when Chinese is active and `中文` when English is active

## Visual Direction

The visual system should move away from heavy industrial black and toward a clearer liquid-glass workbench.

Token intent:

- background: soft layered gradients, not a flat fill
- surfaces: translucent white or warm gray glass panels
- borders: thin and low-contrast
- accent: one warm accent per channel, but muted
- typography: strong title, compact supporting text
- density: medium, not sparse, so the app still feels like a tool

The layout should still look like W4DJ, but it should feel easier to scan.

## Responsive Behavior

### Wide Windows

- left rail stays fixed-width
- two channel cards stack vertically
- logs expand inside each card without breaking the page

### Narrow Windows

- left rail collapses above the cards or becomes a full-width header block
- channel cards stay stacked
- global controls remain accessible without horizontal overflow

### Short Windows

- reduce vertical spacing between blocks
- compress card padding slightly
- keep the action button and path fields readable
- preserve access to logs through the drawer

## Copy Changes

The new design should preserve the current language system, but the copy itself needs to be simpler and more demo-friendly.

Recommended copy strategy:

- keep the title short
- keep the directory labels explicit
- keep mode descriptions in plain language
- keep fallback text short and direct

## Implementation Scope

The code change should be limited to:

- `app/src/app.ts`
- `app/src/styles.css`
- `app/src/app.test.ts`

If a new structural wrapper is needed, it should be added inside the existing frontend render path rather than by introducing a framework.

## Testing Strategy

- verify both sync channels still render
- verify the language switch still persists in `localStorage`
- verify the top-right language control stays in place
- verify mode switching still works in both languages
- verify slot 2 fallback output still displays correctly
- verify the layout does not overflow on narrow or short windows
- verify the logs drawer still expands and collapses per slot

## Acceptance Criteria

- The interface reads as a workbench, not a machine panel.
- The left rail contains the global controls and guidance.
- The two sync channels remain equally visible.
- The EN / 中文 toggle stays in the top-right corner.
- The selected layout looks cleaner on first launch than the current dark-heavy version.
- Existing sync behavior, translations, and fallback logic continue to work.

## Decision

Proceed with B as the design baseline. Use the current dual-slot functionality, but restyle the shell into a glass workbench with stacked cards and a quieter global control rail.
