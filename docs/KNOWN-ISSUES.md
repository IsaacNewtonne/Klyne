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
