# Terminal workbench

Page override for the existing Cranibox design system, using UI/UX Pro Max guidance
on keyboard navigation, mobile input, focus restoration and 44px touch targets.
The interaction reference is devswha/herdr-web-ui at
`b76aebefded791d3416d1df64337e2b76543b217`.

## Layout

- Desktop: a global floating workbench with a 240px session sidebar. Preserve
  the current page behind it. Drag the title bar to move; drag edges or corners
  to resize. Provide maximize, size presets, centering and alignment actions.
- At a window width of 900px and below: sessions move into a modal drawer. The selected terminal
  remains visible; horizontal scrolling belongs only to tabs and shortcut keys.
- A second terminal can be pinned beside the selected tab on desktop. On smaller
  screens it remains connected and can be selected through the tab strip.
- Fit the workbench to the visual viewport, including keyboard resizing and safe
  area padding. Use contained scrolling for session lists, popovers and terminal
  output.

## Interactions

- Selecting a session opens and attaches it immediately. Creating opens a small
  name dialog; the working directory is an optional advanced field.
- Each of up to four open tabs owns its xterm display and WebSocket. Switching
  tabs keeps their attachments and scrollback. Closing a tab detaches; ending a
  session requires a separate confirmation.
- Minimize to the top right toolbar. Keep the workbench mounted and attached
  while hidden and across page navigation. Restore from either terminal entry.
  Closing the window detaches its clients but keeps persistent sessions running.
- Restore open tabs and window state from session storage on reload; store
  desktop geometry in local storage. Legacy terminal URLs still select the
  machine and session. Terminal navigation itself leaves the current page intact.
- Supply arrow-key movement and resizing, with Shift for larger steps, plus
  menu alternatives to pointer dragging. Return focus to the toolbar on minimize
  or close; hidden windows must be inert and excluded from keyboard navigation.
- Use a native popover for menus so scrolling containers and modal drawers do
  not clip them. Support arrows, Home, End, Escape and return focus to the
  trigger. Use a roving tab stop with arrow navigation for terminal tabs.
- On mobile, compose whole commands in a native textarea. Enter during IME
  composition never submits; Shift+Enter inserts a line break. Provide explicit
  Send plus Esc, Tab, Ctrl+C and arrow shortcuts.

## Visual treatment

Use existing dark/light semantic colors and lime accent. The terminal display
keeps its dark ANSI canvas in both themes. Lucide icons use visible labels or
accessible names; selected tabs use an underline, and statuses include text.
Touch controls are at least 44px. Preserve visible focus rings and reduced
motion support from the application stylesheet. Avoid decorative animation.

## Scope

Adapt the reference workbench to the Cranibox session API. This does not expose
the upstream full machine protocol, native tab tree, agent chat, file browser
or approvals UI. No new frontend dependency or additional web server is needed.
Attribution and the upstream MIT notice are included in
`docs/licenses/herdr-web-ui.txt` and the generated web bundle.
