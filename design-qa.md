# GUI design QA

## Comparison target

- Source visual truth: `UI-reference/search.png` and `UI-reference/detail.png`
- Rendered implementation: `apps/desktop/design-evidence/gui-search-synthetic.png` and `apps/desktop/design-evidence/gui-detail-synthetic.png`
- Viewport: 1440 x 900, light theme, device scale factor 1
- State: public-synthetic hybrid query with four reference-density cards visible; first result opened in the local detail sheet

## Full-view comparison evidence

The source and implementation captures were normalized to the same viewport and compared in paired full-frame composites. The implementation preserves the 240px sidebar, 48px top bar, search header boundary, four-card viewport, 12px card rhythm, white/gray surface balance, reference borders, primary indigo, semantic state colors, 14px card radius, 576px detail sheet, 20% overlay, blur, and left-side sheet shadow. Pagination mounts exactly four cards, keeping the buffer below the eight-card contract.

The following product-backed differences are intentional rather than design drift:

- unsupported `导入进度`、`后台任务`、`性能` and `设置` destinations are absent;
- the prototype-only scenario selector is replaced by a real daemon-status pill;
- cards use the bounded fields and exact `SearchSelection` supplied by `resume-ir.search-response.v3` instead of inventing employer, education, or location values;
- unsupported open/reveal/reprocess/delete footer actions are absent from the detail sheet.

None of these differences changes the selected palette, spacing system, corner treatment, density, overlay behavior, or visual hierarchy.

## Focused-region comparison evidence

Focused paired crops were inspected for the search execution bar and first three cards, and for the complete 576px detail sheet. Small text, icon alignment, tag padding, card borders, sheet header, two-column field grid, file panel, normalized-body surface, overlay opacity, and scroll treatment remain readable and aligned with the source design. Focused crops were required because the full paired frame reduces small UI text below a reliable review size.

## Required fidelity surfaces

- Fonts and typography: Geist-compatible stack with native Chinese UI fallbacks; source sizes, weights, line heights, truncation, and hierarchy are preserved. No P0-P2 mismatch.
- Spacing and layout rhythm: 240px sidebar, 48px header, 24px content gutters, four-card viewport, 12px card gaps, 14px card radius, 576px sheet, and reference overlay/elevation are aligned. No P0-P2 mismatch.
- Colors and visual tokens: exact reference tokens are used for foreground, card, primary, muted, border, sidebar, success, warning, error, and info states. No gradients were introduced. No P0-P2 mismatch.
- Image quality and asset fidelity: the target contains no photographic or decorative raster assets. The visible `IR` mark and Lucide interface icons match the supplied reference system; no placeholders or approximate custom drawings are present.
- Copy and content: prototype-only copy was removed. Remaining copy describes real search, filter, import, diagnostics, and local-detail behavior; synthetic capture content is explicitly non-private.

## Findings

- No actionable P0, P1, or P2 findings remain.
- P3: native Chinese glyph metrics vary slightly by operating-system font availability; the fallback stack keeps line wrapping and component geometry stable.

## Comparison history

- Pass 1: full and focused search comparisons found no P0-P2 visual mismatch. The fourth card is intentionally partially visible at the bottom edge, matching the approved four-card viewport contract.
- Pass 1: full and focused detail comparisons found no P0-P2 visual mismatch. Product-capability differences are confined to unsupported prototype actions and unavailable search-card fields; sheet geometry and styling remain aligned.
- Pass 2: after enforcing a fixed 174px card height, two-line snippet clamp, and four-card pagination, fresh paired search/detail captures showed no density, wrapping, spacing, or overlay regression. No P0-P2 finding remains.

## Verification notes

- Chrome rendered both public-synthetic states from the live Vite app without page/runtime console errors; stderr contained only a macOS headless-process policy warning.
- Search controls, filter controls, result selection, overlay close behavior, import actions, diagnostics actions, and automatic detail hydration are wired to production handlers. The selected-result preview exercised the open-detail transition and sensitive local body/path rendering without private data.
- The screenshots are public-synthetic design evidence only. They are not private GUI/manual acceptance, performance evidence, release evidence, or a readiness claim.

## Implementation checklist

- [x] Preserve reference palette, radii, shadows, overlay, scrollbar, hover, focus, and selected states.
- [x] Keep exactly four large result cards visible at 1440 x 900 with scroll navigation.
- [x] Remove unsupported navigation and prototype-only controls.
- [x] Route field filtering through the existing daemon filter contract.
- [x] Automatically hydrate selected local detail with bounded page requests.
- [x] Keep sensitive detail out of logs, diagnostics, benchmark reports, and screenshots.

final result: passed
