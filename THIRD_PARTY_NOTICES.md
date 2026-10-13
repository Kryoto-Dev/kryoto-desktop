# uBlock Origin

Windows download pages use the unmodified official Chromium build of uBlock
Origin 1.75.0 by Raymond Hill and contributors, licensed under GPL-3.0.

- Release and source: https://github.com/gorhill/uBlock/releases/tag/1.75.0
- Archive: `uBlock0_1.75.0.chromium.zip`
- Archive SHA-256: `393cf95709d1074d4022970e9014e434395c53a822387f1e25f43be97cf4b582`
- Extension source, bundled filters and license: `src-tauri/resources/ublock/`

The extension runs in a separate WebView2 profile used only for mirror download
pages. The launcher and Store do not load it. Linux keeps its manual browser fallback.
