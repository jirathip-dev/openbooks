## Summary

Decision: openbooks gets its own look, a light and dark scheme pair and a style file, and every page from round 2 is redrawn in it with no layout change. The accent is a ledger-ink blue on a warm paper ground (a dark warm ground at night). Money is set in tabular figures, corners are softly rounded (6px) and the density is a little tighter. Both schemes pass Blocks' own contrast gate with no misses.
Screens: Today; the receipt (default, and one invoice opened); Documents (all, and Invoices); one invoice opened with its entry; the invoice form; the accountant's book view. Each is drawn light and dark, at 1440 and 390.
Open questions: none. Round 2's two questions are settled as drawn: withholding tax returns stay a Today to-do that the Thai pack adds, and a draft shows "Draft" in the Number column with no prefix.

# openbooks default UI, round 3 (approved)

These are the approved round 3 pages: openbooks' default pages drawn in its own scheme pair and style. They are the reference for the screens issues. The scheme and style files are in this repo's [`web/theme/`](../../../web/theme/), and `theme.css`, which the pages load, is beside them here. The sample company is the invented Nolla Works Co., Ltd. on 26 Nov 2026; every customer, supplier, address and tax ID is invented, and the numbers agree across every page.

## The theme (`web/theme/`)

| File | What it is |
|---|---|
| `light.json`, `dark.json` | Blocks schemes: the 19 colour tokens each. The ground is warm paper (`#f4efe4`, card `#fffcf5`) and at night warm dark (`#1a1814`, card `#23201b`). The accent `--l`/`--link` is ledger-ink blue (`#24497a`, dark `#9fb8e3`). The state colours are moss, ochre and brick, warmed to sit on paper. |
| `style.json` | Blocks style, format 2: `radius` 0.5 (6px), `space` 0.9 (a little denser), `easing` calm, `fills` tint, `sections` quiet. The approved style also sets `buttons` filled (the primary button is filled with the accent); the Blocks version this repo uses has no `buttons` field yet, so `web/theme/style.json` leaves it out until it does, and `theme.css` here shows it as drawn. |
| `theme.css` | What Blocks' own `theme::css` writes from those three files. The mockups load it, and `openbooks.css` adds layout only: every colour, radius and spacing it uses is a token. |

**Gate run.** The scheme pair and style were parsed into Blocks' own `Scheme` and `Style` types, and Blocks' `theme::gate` was run on each scheme with the style's fills. The check was a scratch binary against the blocks crate at 127042f9b; blocks has no CLI command for it. Result: `openbooks light: 0 misses`, `openbooks dark: 0 misses`, exit 0. As a control, the same binary on blocks' refused Solarized fixture reports 20 misses and exits 1.

**Tabular figures.** Blocks' style file has no tabular-figures option yet (blocks issue B3 in the ADR). Until it does, `openbooks.css` sets money and document numbers in the sans face with `tabular-nums lining-nums`, so columns line up without a monospace font. Round 2 used monospace.

**The printed paper** on `04-document` keeps the print's own black ink on white in both modes. It stands in for the PDF template's raster, not for a screen surface, and is the one place the page has colours outside the scheme.

## Images (`png/`, each `-light-` and `-dark-`, at 1440 and 390)

The `png/` images are the renders of the pages in this folder:

| PNG | Decision it shows |
|---|---|
| `01-today-*` | Today in the new look: the Ask box is outlined in the accent with a filled Ask. Below it is the to-do list, whose last item, "Withholding tax returns due 7 Dec", is the Thai pack's to-do (kept as drawn, per q3964). Then three money tiles in tabular figures, and the draft with one accent-filled **Confirm**. |
| `02-receipt-*` | Receipt, default: customer and amount (the big figure in tabular sans, outlined in the accent). The rows settle the oldest invoices first, under one defaults row with Change. |
| `02-receipt-open-*` | Receipt with one invoice opened: full or part (an accent segmented control), tax withheld, adjustment lines with reasons, then Close in full and Done. |
| `03-documents-*` | One Documents table across the five books, with rounded type chips (the current one filled in the accent). Drafts read "Draft" in the Number column, with no prefix (q3964). A void keeps its number, struck through. |
| `03-documents-invoices-*` | Documents filtered to Invoices, with due date, open amount and the month total. |
| `04-document-*` | One invoice as it prints, with its actions (Record payment filled in the accent) and the entry shown below. |
| `05-invoice-*` | Invoice form: customer plus one item line. Everything else is defaulted, with More details and Show entry folded. |
| `06-book-rv-*` | The accountant's book view, RV 02 Receipts, with debits first and credits indented. A void is kept but not counted, and the book balances. `06-book.html` draws every book from its address (`#jv`, `#rv`, `#pv`, `#ap`, `#ar`). |

Rendered with Playwright's Chromium, light and dark by `prefers-color-scheme`. The 390 images are at 2x, with the viewport set to the page's full height so the phone's fixed bars sit at the bottom.
