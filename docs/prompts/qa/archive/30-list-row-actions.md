# QA 30. List rows: push the action to the right edge

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

Seen on New service > GitHub: each repository's **Deploy** button sits right after the text, so the buttons start at a different x on every row and look scattered. They are also big, bright primary buttons with a glow, ten of them in a column.

Cause: `web/css/app.css` has `.list-row>span:first-child{flex:1 1 0;min-width:0}` but the first child in `web/js/pages/newservice.js` (about line 23) is a `div`, so it never grows; `.list-row` also has `flex-wrap:wrap`, so on narrow widths the button drops under the text.

## Check
1. `grep -rn 'class="list-row"' web/js` and list every page that uses it (repos, deploy keys, domains, backups, databases, settings). Note whether the first child is a `span` or a `div`, and where the action sits.
2. Open each at 1440 and 390 px: actions must share one right edge.

## Fix
1. Change the rule to `.list-row>:first-child{flex:1 1 0;min-width:0}` and `.list-row>:last-child:not(:first-child){margin-left:auto;flex:none}`; keep `flex-wrap:nowrap` above 520 px and wrap only below it.
2. Long names and metadata truncate or wrap inside the first child, never push the action.
3. Row action buttons are `btn sm` (min-height 32 px desktop, 44 px on `pointer:coarse`). In the repo list use the secondary style for "Deploy"; keep primary only for the single main action on a page. Remove the shadow glow from small primary buttons (one shared `.btn.sm.primary` rule).
4. Add the same row padding (`12px 0`) so rows are not cramped.

## Pass when
GitHub repos, Deploy keys and one more list use the same layout: text on the left, action aligned right on every row, at 1440, 768 and 390 px. Screenshots in `docs/screenshots/qa30/`.
