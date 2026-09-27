---
id: DESIGN-001
type: design
title: AI Context OS — Design System
status: active
version: 0.1.0
created: 2026-09-27
updated: 2026-09-27
---

# DESIGN

The design system covers two surfaces: **CLI output** (built now) and the **web dashboard**
(deferred to Phase 8, designed now so the CLI's visual language survives the jump).

Principle: *an AI-controlled system must make state legible at a glance.* A human must be able to
tell, without reading prose, whether something succeeded, needs approval, or was denied.

---

## 1. Design principles

1. **State before content.** The first thing rendered is status, then identity, then detail.
2. **Severity is never ambiguous.** Every finding has exactly one severity, one symbol, and one
   colour. The same severity always looks the same.
3. **Colour is redundant, never load-bearing.** Every colour-coded state also carries a symbol and
   a word. Roughly 1 in 12 men has a colour-vision deficiency; a permission decision must never
   depend on hue.
4. **Progressive disclosure.** Summary first, detail on demand (`--verbose`, `--explain`,
   `aicontext audit show <id>`).
5. **Destructive actions are visually loud, approval actions are visually calm.** Blinking or red
   everything makes the real danger invisible.
6. **Copy the source, do not paraphrase it.** Paths, IDs, tool names, and arguments are shown
   verbatim so a human can verify them.
7. **No silent truncation.** If output is cut, say so and say how to get the rest.
8. **Offline and fast.** Every surface works with no network and no spinner longer than 100 ms.

---

## 2. Colour palette

Tokens are semantic. Components reference tokens, never raw hex.

### 2.1 Neutral ramp

| Token | Light | Dark | Use |
|-------|-------|------|-----|
| `surface-canvas` | `#FFFFFF` | `#0B0E13` | Page / terminal default background |
| `surface-raised` | `#F7F8FA` | `#161A20` | Cards, table headers |
| `surface-sunken` | `#EEF1F5` | `#11151B` | Code blocks, wells |
| `border-subtle` | `#DDE2E9` | `#262C35` | Dividers, table rules |
| `border-strong` | `#C2C9D4` | `#3A424D` | Inputs, focus containers |
| `text-primary` | `#161A20` | `#EEF1F5` | Headings, values |
| `text-secondary` | `#4E5763` | `#98A2B3` | Labels, metadata |
| `text-muted` | `#6B7480` | `#6B7480` | Hints, timestamps |
| `text-inverse` | `#FFFFFF` | `#0B0E13` | Text on a filled accent |

### 2.2 Accent

| Token | Light | Dark | Use |
|-------|-------|------|-----|
| `accent-default` | `#3563E9` | `#5C8DFF` | Primary action, links, focus ring |
| `accent-hover` | `#2A4FBF` | `#8FB4FF` | Hover |
| `accent-active` | `#23409A` | `#3563E9` | Pressed |
| `accent-subtle-bg` | `#EDF1FE` | `#16233F` | Selected row, active nav |
| `focus-ring` | `#3563E9` | `#8FB4FF` | Keyboard focus indicator |

One accent. No second brand hue, no gradients in product surfaces.

### 2.3 Semantic state tokens

These carry meaning across the whole system and must never be redefined locally.

| Token | Meaning | Light fg / bg | Dark fg / bg | Symbol | CLI colour |
|-------|---------|----------------|--------------|--------|------------|
| `state-allow` | Permitted automatically | `#1E8E5A` / `#E6F5EE` | `#4ED09A` / `#0F2A20` | `✓` | green |
| `state-approval` | Needs human approval | `#B26A00` / `#FDF3E2` | `#F0B45E` / `#2A2010` | `?` | yellow |
| `state-explicit` | Destructive, per-action consent | `#C2410C` / `#FDEDE3` | `#FB9A6B` / `#2C1710` | `!` | red |
| `state-deny` | Refused | `#C4342B` / `#FBEAE9` | `#FF8A80` / `#2E1414` | `✗` | red |
| `state-info` | Informational | `#2A6F97` / `#E8F2F8` | `#7FC3E8` / `#0F2531` | `i` | blue |
| `state-stale` | Needs attention, not an error | `#6B7480` / `#EEF1F5` | `#98A2B3` / `#1A1F27` | `~` | grey |

`state-explicit` and `state-deny` share the red family but differ in symbol, label, and weight —
the distinction is "you can still say yes" versus "no".

### 2.4 Contrast requirements

- Body text: **≥ 4.5:1** against its background (WCAG AA).
- Large text (≥ 18.66 px bold / 24 px): **≥ 3:1**.
- Icons, borders, focus rings, chart marks: **≥ 3:1**.
- The pairs above are **targets pending automated verification**; a contrast test runs in CI for
  every token pair and fails the build below threshold.
- Never encode state in a hue alone; never use a hue that collides with the adjacent surface.

---

## 3. Typography

### 3.1 CLI

- **Font:** the terminal's own monospace. No font choice, no ligatures, no colour gradients.
- **Size:** whatever the terminal uses. Design for 80 columns minimum; degrade gracefully to 60;
  never require more than 100.
- **Weight:** bold only for status tokens and the primary label of a block. Never bold body text.
- **Rules:** no italics in plain terminals (often rendered as a colour change). No colour inside a
  path, ID, or command — colour those as whole tokens only.

### 3.2 Dashboard

- **UI font:** system stack — `Inter, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial,
  sans-serif`. No webfont download; offline is a hard requirement.
- **Mono font:** `ui-monospace, "JetBrains Mono", "Cascadia Code", Menlo, Consolas, monospace`.
- **Scale** (1.25 ratio, 4 px rhythm):

| Token | Size / line height | Use |
|-------|--------------------|-----|
| `text-xs` | 12 / 16 | Metadata, timestamps, table micro-labels |
| `text-sm` | 14 / 20 | Secondary text, table body, help |
| `text-base` | 16 / 24 | Body |
| `text-lg` | 20 / 28 | Card titles, section headings |
| `text-xl` | 24 / 32 | Page titles |
| `text-2xl` | 32 / 40 | Metric values |
| `text-3xl` | 40 / 48 | Rare, single-metric focus |

- **Measure:** 60–80 characters. Long-form content is capped at `72ch`.
- **Case:** sentence case for headings and labels. ALL CAPS only for 10 px metadata labels.
- **Numbers:** tabular figures for anything in a column, so digits align.

---

## 4. Spacing, grid, radius, elevation

**Base unit 4 px.** Only these steps are permitted: `4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 80`.

| Token | Value | Typical use |
|-------|-------|-------------|
| `space-1` | 4 px | Icon-to-label, badge padding |
| `space-2` | 8 px | Between related labels and values |
| `space-3` | 12 px | Inside a compact control |
| `space-4` | 16 px | Card padding (mobile), list gaps |
| `space-6` | 24 px | Card padding (desktop), section gaps |
| `space-8` | 32 px | Between major sections |
| `space-12` | 48 px | Page rhythm |
| `space-16` | 64 px | Above a page title |

- **Grid:** 12 columns, 16 px gutter, max content width 1200 px, 24 px page margin (16 px under
  768 px). Sidebar 240 px, collapsible to 64 px.
- **Radius:** `4 px` (inputs, badges), `8 px` (buttons, cards), `12 px` (modals, panels),
  `9999 px` (pills, avatars). Never more than 12 px except pills.
- **Elevation:** prefer a border over a shadow. Levels: `0` none, `1` `0 1px 2px rgba(0,0,0,.06)`
  (cards), `2` `0 4px 12px rgba(0,0,0,.10)` (dropdowns), `3` `0 12px 32px rgba(0,0,0,.16)`
  (modals). In dark mode, elevation is expressed with surface lightness, not shadow.
- **Motion:** 120 ms for hover, 200 ms for enter/exit, ease-out. Respect
  `prefers-reduced-motion: reduce` by removing transforms and keeping opacity at 0 ms. No animation
  conveys meaning on its own.

---

## 5. Components

Reuse before creating. A new component requires a justification in the change description, and
`doctor`-style checks flag near-duplicates.

### 5.1 Surfaces

| Component | Contract |
|-----------|----------|
| `Card` | `surface-raised`, 1 px `border-subtle`, radius 8, padding `space-6`. Header row is title + optional actions, separated by a divider |
| `Panel` | Full-width grouping with an optional sticky header |
| `Banner` | Inline, full-width message. Tones map 1:1 to `state-*` tokens. Icon + title + optional action. Never dismissible if it reports a deny |
| `EmptyState` | Icon + one-line explanation + one primary action. Never a bare "No data" |
| `Callout` | Compact note inside body text. Tones map to `state-*` |

### 5.2 Controls

| Component | Contract |
|-----------|----------|
| `Button` | Variants: `primary` (filled accent), `secondary` (border), `ghost` (text only), `danger` (`state-deny`, filled, requires a confirm dialog). Sizes: `sm` 28 px, `md` 36 px, `lg` 44 px. All interactive targets ≥ 44 × 44 px in dashboard, ≥ 28 px in dense tables |
| `Input` / `Textarea` / `Select` | Radius 4, 1 px `border-strong`, focus = 2 px `focus-ring` at 2 px offset, error = `state-deny` border + message below, never colour alone |
| `Checkbox` / `Radio` / `Switch` | Label is always visible; the control is not the label. Switch is for immediate effect, checkbox for form submission |
| `FilterBar` | Search + facet chips above a table. Selected filter uses `accent-subtle-bg`, and shows a remove affordance |
| `CodeBlock` | `surface-sunken`, mono, `text-sm`, line numbers optional, copy button on hover, long lines wrap with a visible continuation marker |
| `DiffView` | Added `#E6F5EE`/`#1E8E5A`, removed `#FBEAE9`/`#C4342B`, gutter marks `+`/`-`. Never colour alone — the glyph is authoritative |

### 5.3 Data display

| Component | Contract |
|-----------|----------|
| `DataTable` | Sticky header, sortable columns show a direction glyph, tabular figures, row hover, keyboard navigable, empty and loading states designed, no horizontal scroll below 768 px (switch to stacked cards) |
| `KeyValue` | Definition list for metadata; label `text-secondary text-sm`, value `text-primary`, values wrap rather than truncate silently |
| `StatusBadge` | One of the `state-*` tokens, always icon + label, never a bare dot |
| `ProgressBar` | Determinate only, with a visible numeric value for assistive technology |
| `Tree` | Collapsible file/ADR/task tree; keyboard operable; expansion state in the URL |
| `Timeline` | Vertical, for audit and change history. Monotonic timestamps, actor and action always visible |
| `MetricCard` | `text-2xl` value, `text-sm` label, delta with an explicit direction word, never a bare percentage |

### 5.4 Navigation and feedback

| Component | Contract |
|-----------|----------|
| `Nav` / `Sidebar` | Current item marked with `aria-current` **and** a 3 px accent bar — position, not colour, is the primary signal |
| `Tabs` | Underline the active tab; `role="tablist"` with arrow-key navigation |
| `Modal` | Focus trapped, `Esc` closes unless destructive, restore focus on close. Destructive modals name the exact target and require typing the resource name to confirm |
| `Toast` | Transient, 4 s, for success only. Errors, denials, and approval requests are **never** toasts — they are banners or modals |
| `Tooltip` | Supplementary only. Never contains information unavailable elsewhere |
| `Skeleton` | Only for content whose shape is known; otherwise a progress indicator. No layout shift on load |

---

## 6. CLI output design

The CLI is the primary surface today, so it has its own contract.

```text
$ aicontext doctor

AI Context OS  ·  my-project  ·  branch feature/context-engine

  ✗ error    ARCHITECTURE.md names MongoDB; project config indicates PostgreSQL
  ✗ error    TASK-014 references SPEC-auth-missing, which does not exist
  ⚠ warn     ADR-004 is marked deprecated and has no successor
  ✓ ok       RULES.md valid
  ✓ ok       TASKS.md valid  (24 tasks, 3 in progress)

  2 errors, 1 warning  ·  exit 3
  next: aicontext doctor --explain
```

**Rules**

- Severity glyph first, then a fixed-width severity word (`error` / `warn` / `ok`), then the message.
  The layout is column-aligned so the eye can scan the glyph column.
- The header line is always `product · project · branch` — identity before content.
- Every run ends with a summary line: counts, exit code, and the single most useful next command.
- Never print a spinner for work under 100 ms. Beyond 100 ms, use a progress line on stderr.
- Colour is opt-out: honour `NO_COLOR`, and disable colour automatically when stdout is not a TTY
  or `TERM=dumb`. `--color=always|auto|never` overrides.
- Symbols degrade: `✓` → `+`, `✗` → `x`, `⚠` → `!` when the terminal is not UTF-8.
- Tables adapt to width; below 60 columns they switch to a two-line-per-record layout. Never
  truncate a path silently — wrap it, or end with `…` and print the full value under `--verbose`.
- `--json` emits a single object to stdout with a documented, stable shape, and all human text goes
  to stderr.
- Exit code is always shown, because scripts branch on it and humans debug with it.

---

## 7. Accessibility

- **Contrast:** verified per §2.4 in CI, not by eye.
- **Focus:** visible on every interactive element, `:focus-visible` only, 2 px ring at 2 px offset,
  never removed without an equal replacement.
- **Keyboard:** every action reachable and operable by keyboard. Dialogs trap and restore focus. The
  CLI is keyboard-free by nature but must stay usable with a screen reader reading plain lines, so
  it never relies on box-drawing characters for meaning.
- **Screen readers:** every icon-only control has an accessible name. Status is announced via a
  live region for approvals and denials. Tables use real `<th>` with `scope`.
- **Motion:** honour `prefers-reduced-motion`. No parallax, no auto-advancing carousel, no motion
  that delays content.
- **Zoom:** layout survives 200 % zoom and 320 px width without horizontal scrolling of the page.
- **Language:** plain, literal wording. No idiom, no pun, no exclamation. Error messages say what
  happened and what to do.
- **Time and units:** absolute timestamps with an explicit timezone in audit output; relative times
  are supplementary and always paired with the absolute value.

---

## 8. What this system does not do

- No dark-mode-only design. Dark is a first-class token set, not an inversion.
- No glassmorphism, no neon, no gradient mesh, no animated backgrounds.
- No motion used decoratively.
- No bespoke illustration. One geometric mark for the product, used sparingly.
- No component that exists only to look designed. Every element earns its place by carrying
  information the developer needs.
