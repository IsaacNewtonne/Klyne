# Known issues

## Multi-step browser click tasks are unreliable with qwen3.5:9b

Test: "Open https://example.com in the browser, click the 'Learn more' link, and tell me the title of the page it opens." Three live runs (2026-10-25):

- 1 of 3 fully correct ("Example Domains").
- One run completed but answered "title unavailable".
- One run ended Blocked after looking for a second 'Learn more' link on the page it had already landed on.

The click itself works: `browser_open` and `browser_click` reach the right page each time. The problem is the planning after the click. The model loses the original goal and tries to repeat the click instead of reading the new page.

Ideas for the fix:

- Keep the original goal in the post-click context, so the model knows it is now on the target page.
- Stop re-planning the click once the page has changed (URL or title differs from before the click).
- Extract the title from the landed page automatically and include it in the click result.

Status 2026-11-01 (branch `fix/post-click-observation`, d5dd72b): fixed for this test in live runs against qwen3.5:9b. Cause: the compact context clipped each observation at 3,000 characters, and the stored JSON was alphabetical, so on the 3,842-character iana.org click result the `title` and `url` keys came after the clip. Fix: head-first observations (url, title, changed, url_before, clicked, note; text; links last), a compact view with text and links cut separately, a Klyne-written click note, a retry guard for clicks that already landed, and snapshot ids on link refs.

- Before (9329406 build), 5 runs: all ended Completed with "Example Domains", but none was clean. One needed an interaction-check repair. One fetched instead of clicking first. Three looped: 35, 70 and 103 steps; 167, 403 and 806 s; 148k to 504k prompt tokens; loop warnings.
- After, 5 runs: 5 of 5 clean. Each made one click, and the worker answered "Example Domains" on its first answer. They took 7 to 12 steps, 30 to 70 s and 19k to 44k prompt tokens.
