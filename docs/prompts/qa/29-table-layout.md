# QA 29. Table layout: padding, scrollbar, columns and alignment

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up). CSS only; no change to the `table()` helper's signature.

Seen in the panel (Containers, System > Updates security list):
- The first column has no left padding, so its text sits flush with the edge of the header band.
- A horizontal scrollbar shows under tables that already fit.
- On wide screens the spare width goes into one big gap (between State and CPU); columns do not feel planned.
- The Updates table: the checkbox column ("Select") is as wide as a text column and the header does not line up with the boxes.

Likely cause: `web/css/app.css` defines the table in several layers that fight each other: `.tw{overflow-x:auto}` three times (lines about 83, 179, 403), `th`/`td` padding set four times (`padding:...0 16px 0 0`, `td,th{padding-right:20px}`), two `td{height}` values, two `th` font rules, and `th.r,td.r` twice. Padding is only on the right, never the left.

## Check
1. Open Containers, Updates (System), Apps, Backups and Audit at 1440 px and 1024 px. List which of the four problems above each page shows.
2. Open DevTools on Containers: find what makes `.tw` overflow (usually the actions column plus `white-space:nowrap`, or `th` padding). Note the `scrollWidth - clientWidth` number.

## Fix (CSS in `web/css/app.css`, remove duplicates, do not add a new layer)
1. Keep ONE table block (the 16d one near line 400) and delete the older duplicate rules for `th`, `td`, `.tw`, `.r`. Net change should be fewer lines.
2. Cell padding `12px 16px`; first cell `padding-left:16px` and last cell `padding-right:16px`, so text never touches the band edge. Header band keeps its background and radius.
3. `.tw{overflow-x:auto}` stays, but the table must not overflow when it fits: use `table{width:100%;table-layout:auto}` and put `min-width:0` on the first column's content so long names truncate with an ellipsis instead of widening the table. The action column is `width:1%; white-space:nowrap`.
4. Column intent, by class, not per page: name column takes the remaining width (`width:auto`), numeric columns `.r` have `width:1%` and `white-space:nowrap`, so spare width goes to the name, not to a gap in the middle.
5. Checkbox column (`.td-check`, add if missing in `table()` callers by class only): `width:44px`, centered, header cell empty with a `sr` label "Select", and a select-all box in the header on Updates.
6. Header text and cell text share the same left or right edge in every column.

## Pass when
Screenshots at 1440 and 1024 px of Containers and Updates show: text clear of the band edge, no scrollbar, no mid-row gap, checkbox column narrow and aligned with its header, header and values aligned. Save before and after in `docs/screenshots/qa29/`. Check Apps and Backups did not break (same screenshots).
