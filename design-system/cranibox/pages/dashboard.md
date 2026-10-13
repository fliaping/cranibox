# Cranibox browser control center

This override applies to all eight browser management pages. The generated
MASTER.md supplies the dashboard style; the product is an Agent workspace,
not a smart home product. Omit landing-page sales sections and demo metrics.

- Layout: fixed navigation, page title, real status summary, task controls,
  then details. Keep operational screens dense but allow 24–40px page gutters.
- Color: slate surfaces (#0f141c, #171e29) and lime actions (#a3e635 with
  #152008 text). Reserve green, amber and red for semantic status. Light mode
  uses white cards on #f5f7fa with dark text. Status always includes words.
- Type: use locally installed Fira Sans/Fira Code when available, followed by
  system sans-serif and monospace fonts. No font download or third-party CDN
  is required, so the console works behind restricted gateways.
- Behavior: retain route, filters, focused fields and entered configuration
  during polling; show the last successful update and stale state on failure.
  Slow polling while hidden. Keep cancel and emergency stop available during
  jobs and configuration dialogs. Show logs only from actual operations.
- Accessibility: visible keyboard focus, labelled icon controls, native modal
  dialogs with restored focus, a trapped mobile navigation drawer, meaningful
  empty/error states, reduced-motion support, 44px mobile controls.
- Verification sizes: 375, 768, 1024 and 1440px. No page-level horizontal
  scroll. Test both themes and Chinese/English layouts.

Implementation: addons/control-center/ui/src. Build static assets before the
Rust binary so the installed web server does not need a Node process.
