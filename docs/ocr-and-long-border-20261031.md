# OCR and long capture follow-up — 2026-10-31

- OCR results now include an initially selected pin button. Toggling it calls the native always-on-top setter; reopening a result resets both the native state and button to pinned.
- Windows OCR enlarges its recognition input up to 2.5 times using cubic interpolation. Overlapping strips stay within the runtime-reported OCR dimension limit. Midpoints determine overlap ownership so strips do not intentionally duplicate text. Source, pin and saved images retain their original resolution.
- Only spaces between CJK ideographs are removed. Other text is not guessed or replaced. WinRT failures are converted to the existing Flutter error response instead of escaping the standard exception handler.
- Local OCR logging records source dimensions, byte count, elapsed time, result character count and failures, without recording recognized text.
- Long capture guides paint outside the selection. On Windows, the selection overlay is also excluded from capture using the existing capture exclusion helper. If unavailable, the guide is hidden before capture. The outcome is logged as `long-overlay-exclusion` and, when needed, `long-overlay-guide-hidden`.

## Evidence

The user's supplied 842×25585 PNG has RGB (22,136,240) in columns 4–6 for all 25,585 rows. The guide therefore exists in the exported pixels, not only the preview UI.

The 16:03–16:05 session logs show OCR followed by both result windows, and a successful user-stopped long capture at 842×25585, with no recorded crash, registration failure or stitch failure. Long export took approximately 1.3 seconds. These logs do not establish pixel-perfect stitching for every browser movement.

Compiled the actual Windows OCR implementation in a temporary command-line harness and ran it against the retained 857×986 OCR source. For the six visible subtitle occurrences of `正文行号`, baseline recognized one correctly after ignoring spaces; enlarged input recognized all six. This is a targeted comparison, not an overall accuracy percentage. A remaining number split and obscured right-column digits were still imperfect.

The same native implementation successfully processed the user's full 842×25585 image. Recognition still has occasional numeric errors; successful processing is not a claim of perfect transcription.

84 Flutter tests passed, including the native pin-toggle calls, OCR/pin dispatch and failure behavior, and transparent pixels throughout the long-capture selection interior. Static analysis has no errors or warnings, with nine existing style infos in the checked files. Desktop capture exclusion and the exported blue-edge fix still need an interactive browser scroll check in the new Windows package.
