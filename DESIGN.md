# Google Photos Takeout Restorer - Design System (Modern Utility)

## 1. Design Principles
This application is designed as a **Professional Desktop Utility**. The philosophy is grounded in three core principles:
1. **Uncompromised Density (Linear style):** Power users need to see hundreds of file paths and statistics at once. Whitespace is used structurally, not decoratively. Oversized tap targets are discarded in favor of precise, mouse-and-keyboard optimized hit areas.
2. **Instant Feedback (Fluent 2):** Interactions must feel instantaneous. Hover states trigger immediately (100ms) to confirm interactivity. Long-running tasks must provide deterministic progress or pulsing skeletons to confirm system liveness.
3. **High-Contrast Semantics (Vercel minimalism):** Chromatic colors (Red, Green, Blue, Yellow) are reserved **exclusively** for semantic status (Error, Success, Info, Warning). The primary aesthetic relies on monochrome high-contrast to reduce cognitive load and eliminate branding distractions.

## 2. Accessibility & High-DPI Scaling Rules
- **Contrast:** Primary text must maintain WCAG AAA contrast ratio (7:1). Muted text must maintain AA (4.5:1).
- **Focus Indicators:** All interactive elements must exhibit a rigid 2px `border_focus` outline with a 2px offset when navigating via keyboard.
- **High-DPI:** All sizing tokens are expressed in logical pixels (`px` in Slint, which scale automatically with OS DPI settings). Raster images are forbidden for icons; vector SVGs or text-based glyphs must be used to ensure infinite scaling.
- **Theme:** The application defaults to the Operating System's theme preference.

## 3. Design Tokens
Components must NEVER hardcode colors or sizes. They must bind to `Theme.slint`.

### Color Semantics (Auto-switching Light/Dark)
- `bg_app`: Deepest window background.
- `bg_panel`: Base layer for sidebars and content panes.
- `bg_surface`: Elevated interactive layer (Cards, Inputs).
- `bg_hover`: Subtle highlight for interactive bounds.
- `border_subtle`: 1px structural outline.
- `border_focus`: 2px high-contrast accessibility ring.
- `text_primary`: High-contrast body text.
- `text_secondary`: Muted metadata text.
- `text_disabled`: Low-contrast disabled text.
- `text_inverted`: Contrast text used over inverted primary backgrounds.
- `accent_primary`: High-contrast inverted background for primary calls to action.
- `status_error`, `status_success`, `status_warning`, `status_info`: Chromatic semantic indicators.

### Typography Hierarchy (System UI Font)
- `text_xs` (11px, Medium 500): Timestamps, extremely dense tabular data.
- `text_sm` (12px, Medium 500): **Default Body**. Used for lists, file paths, standard labels.
- `text_base` (14px, SemiBold 600): Primary Button labels, sub-headers.
- `text_lg` (16px, Bold 700): Section headers (e.g., "Step 2: Destination").
- `text_xl` (20px, Bold 700): Page titles (e.g., "Welcome").

### Grid & Spacing System (4px Base)
- `space_1` (4px): Gap between icon and text.
- `space_2` (8px): Inner padding for inputs and compact buttons.
- `space_3` (12px): Padding for cards and list items.
- `space_4` (16px): Gap between distinct component groups.
- `space_6` (24px): Standard page padding.
- `space_8` (32px): Major layout divisions (e.g., separating sidebar from content).

### Border & Elevation System
- `radius_sm` (4px): Inputs, compact buttons, badges.
- `radius_md` (6px): Cards, dialogs.
- **Elevation:** Drop shadows are strictly avoided to ensure rendering performance. Hierarchy is defined by background lightness: `bg_app` -> `bg_panel` -> `bg_surface`.

### Icon System
- Icons use a strict 16x16px bounding box for inline text (`text_sm`), and 24x24px for toolbars.
- Stroke width: 1.5px. Monochrome, inheriting `text_primary` or `text_secondary`.

### Animation Tokens
- `duration_instant`: 0ms (Used for focus rings).
- `duration_fast`: 100ms (Hover state transitions).
- `duration_normal`: 200ms (Dialog pop-ins, page transitions).
- `easing_standard`: ease-in-out.

## 4. Component Definitions

### Buttons
- **Primary:** `accent_primary` bg, `text_inverted`, 1px border. Used ONLY for the main "Next" or "Start" action.
- **Secondary:** `bg_surface`, `text_primary`, 1px `border_subtle`. Default action.
- **Ghost:** Transparent bg, `text_secondary`. Highlights on hover. For tertiary actions.
- **Danger:** `status_error` bg, `text_inverted`. For irreversible destructive actions.

### Cards & File Systems
- **Default Card:** `bg_surface`, `border_subtle`, 6px radius. Static container.
- **Interactive File Card:** Includes hover state (`bg_hover`) and dense layout for ZIP/Folder picking. Displays path, size, and a secondary action (Remove).

### Tables
- **Dense Table:** 24px row height. Alternating backgrounds (`bg_panel`, `bg_surface`) for readability without gridlines.

### Inputs
- **Text / Picker:** 28px fixed height, `bg_surface`, `text_sm`. Border transitions to `border_focus` on active.

### Dialogs & Recovery
- **Modal Dialog:** Centers over an 80% opacity `bg_app` scrim. Max width 400px. Used for Fatal Errors and Recovery prompts.
- **Recovery Banner:** A distinct yellow (`status_warning`) full-width banner injected at the top of the Welcome page, rather than a blocking dialog, allowing the user to read context before deciding.

### Loading States
- **Skeletons:** Pulsing background (`bg_surface` to `bg_hover`) over 1000ms duration. Used before table data populates.
- **Progress Bar:** Flat 4px height, sharp corners. Track is `border_subtle`, fill is `accent_primary`.

## 5. Implementation Status
This document serves as the ground truth for Phase B UI Redesign. Components must be verified in `ComponentPreview.slint` prior to integrating with application logic.
