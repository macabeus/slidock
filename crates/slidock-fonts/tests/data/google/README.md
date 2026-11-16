# Recorded Google Fonts `css2` responses

Fetched 2026-10-31 with `curl`; `parse_css2` and `css2_url` are checked against them (`tests/google.rs`). A change in
Google's behaviour shows up when these are re-recorded.

| File | Request | `User-Agent` |
|---|---|---|
| `roboto-cli.css` | `css2?family=Roboto:ital,wght@0,400;0,700;1,400&display=block` | `slidock/0.1` |
| `roboto-browser.css` | same | Chrome 141 (macOS) |
| `oswald-cli.css` | `css2?family=Oswald:wght@400;700&display=block` | `slidock/0.1` |
| `oswald-browser.css` | same | Chrome 141 (macOS) |
| `two-families-cli.css` | `css2?family=Barlow+Condensed:ital,wght@0,700;1,700&family=Roboto+Mono:wght@400&display=block` | `slidock/0.1` |

`css2?family=Oswald:ital,wght@1,400` (a face Oswald does not have) answered HTTP 400 with an HTML page.
