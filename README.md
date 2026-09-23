# rstpd for Linux

A lightweight, native Linux text editor using **GTK3, stock Adwaita, Scintilla
and Lexilla**. This is a Linux-specific derivative of
[Sockolet/rstpd](https://github.com/Sockolet/rstpd), retaining its Git history
and MIT license. The Windows application remains in its original repository.
The starting point is upstream commit `54fa244e963efdc48b1c167f04b7b23225490f5f`.

GTK3 is intentional: the pinned Scintilla supports GTK2/3, not GTK4.
Keeping that editing engine preserves multiple carets, rectangular selections,
Lexilla language coverage and UDL highlighting. This is not a GtkSourceView
rewrite or a GTK4/libadwaita application. GTK supplies the Adwaita controls,
symbolic toolbar icons and dialogs; there is no application CSS theme.
Light, dark and system-preference modes are available. System mode follows
GNOME's color-scheme setting when available, otherwise the initial GTK preference.

No plugins, extension discovery, embedded browser, command
runner, script host, or automatic updater.

The application, document/session management, compare workflow, JSON tools,
search, encoding conversion, and UI integration are written in Rust.
**Scintilla and Lexilla are statically linked C++ editor components**, with a
small C++ adapter for reference-counted document release and GTK notifications.
This is not a pure-Rust implementation of the editing engine. No Notepad++
application code or plugin code is included.

## Run

Download the Linux x86_64 archive and its SHA-256 checksum from
[Releases](https://github.com/Sockolet/rstpd-gtk/releases/latest), or build from
source using the instructions below. Verify the archive with
`sha256sum -c rstpd-1.3.0-linux-x86_64.tar.gz.sha256`, extract it, and run
`./rstpd` from the extracted directory. GTK3 must be installed on the system;
the upstream Windows ZIP is not a Linux package.

```sh
./target/release/rstpd
./target/release/rstpd ./example.rs ./example.json
./target/release/rstpd --session-dir "$HOME/.local/state/rstpd-work" ./notes.md
```

Use `--session-dir DIRECTORY` for a separate workspace. Only one
instance may use a session directory at a time. Opening another instance
does not forward filenames to the existing instance. Local files can also be
dropped onto the editor. Remote URLs are not opened or downloaded.
Use `--` before filenames beginning with `-`.

Recovery reads upstream version-1 and version-2 schemas and saves version 2,
including language/completion definitions, font and symbol-display preferences. Do not share a live session directory
between platforms: filenames and locking semantics differ.

## Features

| Area | Implementation |
|---|---|
| Native UI | GTK3 Adwaita menus, dialogs and controls; desktop scaling, light/dark/system preference, closeable tabs and a symbolic icon toolbar |
| Tab management | Native drag reordering, pinned-left tabs, restored order/pins, and double-click after the last tab to create a document |
| External changes | Background monitoring, automatic reload of clean files, and explicit confirmation before discarding unsaved edits |
| Editor font | Native font-family and point-size chooser; per-workspace persistence across tabs and split panes |
| Character counts | Unicode totals beside line/column, or selected characters out of the total for normal, multiple and rectangular selections |
| Show symbols | Independent whitespace, EOL, non-printing/control markers, Show All, indent guides and wrap symbols, saved per workspace |
| Highlighting | All 94 source-language inventory entries mapped, plus the full pinned Lexilla catalog; filename detection, mode selection, and data-only UDL 2.0/2.1 import |
| Markdown | CommonMark/GFM parsing with visible headings, emphasis, links, lists, quotes, tasks, tables, inline/fenced/indented code and strikethrough |
| Completion | Keywords, local declarations, member/context suggestions, common built-in APIs, parameter hints and importable completion signatures |
| Multiple edits | Alt+drag rectangular selection, Ctrl+click multiple carets, multi-selection typing and paste |
| Split screen | Two editable views of the same document or two different documents |
| Document map | Clickable compact view with the visible text range highlighted; scroll the map for long documents |
| Search | Normal, extended and advanced regex, including look-ahead/look-behind and pattern backreferences; case/whole-word options, wrap-around and replacement |
| Find All | Current/all-open-document searches with grouped matches, highlighted snippets and a resizable, navigable bottom panel |
| Find in Files | Folder/filter search, optional recursion/hidden files, background cancellation, and navigable disk results |
| Compare | Live line/character differences, moved-line markers, aligned panes, ignore options, selected-line/clipboard/last-saved comparisons |
| JSON | Lossless JSON/JSON5 pretty-print/minify, automatically refreshed tree, RFC 6901 pointers and source-span navigation |
| Text operations | Upper/lower/title/sentence/inverted case; case-sensitive, case-insensitive, natural and exact decimal sorting; reverse/join, deduplication and whitespace operations |
| Encoding | UTF-8, UTF-16/32 LE/BE, Windows/ISO/OEM code pages, Shift-JIS, EUC-JP, ISO-2022-JP, GBK/GB18030, Big5, EUC-KR, KOI8 and Mac encodings |
| Line endings | CRLF, LF and CR conversion; new Linux documents default to LF |
| Recovery | Background atomic snapshots of all tabs, including unnamed documents; restore after restart or crash |

To choose documents for a split, click a pane and then choose its tab.
The icon toolbar keeps the same New, Open, Save, Find, Split, Compare, JSON tree
and Document map actions. Hover for a descriptive tooltip and shortcut.
Button names remain available to accessibility tools, and keyboard shortcuts
and editing commands are retained. Icons come from the standard GTK icon theme
and adapt to light/dark appearance; no icon font or browser runtime is required.
The right-hand document's tab is marked `[R]`. Drag the divider to resize.
**Compare** compares the active tab with the next tab (wrapping at the end).
Editing either document automatically schedules a comparison refresh.
Old worker results are discarded if the documents changed in the meantime;
refresh does not move your editing caret. EOL representation is ignored during
line comparison.

### Tabs and external changes

Drag tabs to reorder them. Right-click a tab for **Pin tab / Unpin tab** and
**Move tab left/right**, or use **File > Pin / unpin tab**. The movement commands
also use Ctrl+Shift+PageUp/PageDown. Pinned tabs form a left-hand group; neither
dragging nor the movement commands can cross its boundary. Pinning does not
make a document read-only. Order and pins persist in workspace recovery.
Double-click the empty area after the last tab to create an untitled document.

The tab context menu also provides **Open in split view**, which opens the
right-clicked document in the other pane while keeping the active document
in place. It reuses an existing split. **Compare with current view** compares
the document active before the right-click against the clicked tab, not the
next tab: the current document appears on the left and the clicked document
on the right. Comparing a tab with itself is disabled. Both actions reuse
existing documents and their unsaved edits without creating copies.
Opening a normal split clears any active comparison.

**View > Automatically reload external changes** is enabled by default and
saved per workspace. A background worker polls named files about once a second,
with periodic content checks for same-size/same-timestamp rewrites. Clean
buffers reload automatically, preserving selection/caret/scroll where possible.
Unsaved text or encoding/EOL changes require confirmation. Declining keeps
the buffer and the existing save-conflict check; the same rejected disk version
does not repeatedly prompt. Missing/unreadable files are reported, not loaded
as empty documents. This is automatic refresh, not log-follow/tail mode.

### Comparison options and sources

**Tools > Compare options...** configures whitespace, case, empty-line and
regex ignoring, moved-line detection and alignment. Options persist in the
workspace. Ignore options decide which lines differ; inline spans retain
original character coordinates. Red/green markers indicate left/right changes
and blue markers identify matching moved lines. F7/Shift+F7 navigate groups.

Alignment uses visual annotations and leading pane space without inserting text.
It temporarily disables wrapping, restores the wrapping preference on clearing
comparison, and supports at most 20,000 spacer rows. Disable alignment for
larger gaps. Comparison preserves editing carets; stale worker results are
discarded when the source documents or settings change.

**Compare selected lines in both panes** requires two different split documents
with one contiguous selection each and expands selections to complete lines.
Edits end selection comparison; reselect before comparing again.
**Compare with clipboard** and **Compare with last-saved file** create ordinary
untitled snapshot tabs without overwriting source files. Whole-document
comparisons continue updating after edits.

This is independently implemented behavior inspired by
[ComparePlus](https://github.com/pnedev/comparePlus), not ported plugin code.
There is no plugin loading, Git/SVN integration, merge operation or multiple
simultaneous comparison pairs.

**View > Editor font...** selects the default editor font family and size
(4–72 points) using GTK's installed-font chooser. The choice is saved with the
workspace and applies to current and new tabs, both split panes, line numbers
and completion/call-tip text. It does not change the Adwaita interface font.
Confirming resets both panes' temporary zoom; **Reset zoom** subsequently
returns to the selected size. Cancel leaves the settings unchanged.

Syntax bold, italic and underline styling is retained. Explicit font-family
and size overrides in imported language definitions still take precedence.
The document map keeps its compact 2-point text. Old sessions without a font
preference continue to use Monospace at 11 points.

Line operations affect complete selected lines, or the whole document when
there is no selection. Case conversion operates on selected text, including
rectangular/multiple selections. These edits participate in undo.
Next to line and column, the status bar displays `4995 characters` normally or
`852 of 4995 characters` for a selection. Counts follow the active editing pane,
include all selected text without double-counting overlaps, and exclude virtual
space. They count Unicode code points rather than UTF-8 bytes: `U+1F680` is one,
combining marks are separate, and CR/LF count separately. A native document
index keeps totals current without copying/rescanning the whole text on caret moves.
Numeric sorting compares decimal digits without floating-point rounding, so
large integer values remain correctly ordered. Every selected line must contain
a decimal number; invalid input is reported before any edits are applied.

The JSON tree refreshes after a short typing pause and preserves expanded paths.
Incomplete or invalid JSON temporarily pauses navigation instead of pointing
at stale positions. JSON5 formatting keeps comments, quoted/unquoted keys,
trailing commas, hex values and original number spellings; it does not convert
JSON5 into strict JSON. Minification retains comments and required line breaks.

Markdown uses a Rust parser rather than the limited legacy Markdown lexer.
Headings and strong text use bold, emphasis uses italics, links are underlined,
code has a contrasting foreground/background, and strikethrough is drawn over
the source text. Backtick and tilde fences are recognized, including unfinished
fences during editing. Styling refreshes in the background and works in split
views and the document map. This is source highlighting, not a Markdown preview;
embedded language-specific token coloring inside code fences is not provided.

Other lexers use semantic style names, metadata and bundled font attributes.
Types, functions, properties, tags, strings and comments are no longer flattened
into the same generic identifier color, including styles used by embedded markup.

## Language definitions and completion

The Language menu starts with **Plain text**, followed by fixed **A-C, D-F,
G-I, J-L, M-O, P-R, S-U and V-Z** groups. Languages are alphabetized within
each group, with GTK's native scrolling for long menus. Imported definitions
have their own **User-defined** group; import/removal commands are at the bottom.

`assets/language-coverage.psv` records the 94 source-language entries and their
extensions from the upstream inventory with model date July 14, 2026, checked
September 19, 2026. The internal `searchResult` pane format is not a source-file
language. Shared/ambiguous extensions can still be selected explicitly in
**Language**; matching the inventory is not a guarantee of identical token
colors for every dialect.

**Language > Import user-defined language XML** accepts UDL 2.0/2.1 data.
Keyword groups, case/prefix rules, comments, delimiter alternatives/escapes,
nesting, operators, number rules, folding, colors and installed-font settings
are processed by a Rust highlighter. XML entities/DTDs are rejected. Definitions
never load libraries, run scripts, or access resources named inside the XML.
Use **Remove current user-defined language** to remove one without changing text.

Completion uses keywords, nearby code and declaration/signature analysis within
a 64 KiB context window. It recognizes common string/list/map types, local
class methods and common APIs. Type inference is heuristic, not a project-wide
compiler or language server. Common APIs are built-in; other library signatures
can be imported with **Language > Import completion API for current language**
using `AutoComplete / KeyWord / Overload / Param` XML data.
Both kinds of definitions persist in the session.

```sh
./target/release/rstpd --import-language ./language.xml ./example.rstlang
./target/release/rstpd --completion-api ./functions.xml ./example.rs
```

The completion API import is associated with the active file's language.

## Show symbols

**View > Show symbols** provides independent checkable options:

- **Show space and tab**: dots for spaces and arrows for tabs.
- **Show end of line**: CR/LF markers for actual line-ending characters.
- **Show non-printing characters**: named markers for Unicode/non-breaking spaces,
  zero-width characters and directional formatting.
- **Show control characters & Unicode EOL**: named C0/C1 control, DEL, NEL, LS and PS markers.
- **Show all characters**: turns the preceding four options on together, or off
  together when all are already enabled.
- **Show indent guide**: vertical indentation guides.
- **Show wrap symbol**: visual wrap markers when word wrap is enabled.

Show All does not change guide/wrap settings. NEL/LS/PS display when either
non-printing or control display is enabled. Hidden controls use an unboxed
space, remaining selectable. These preferences affect both editing panes and
new tabs, survive theme/font changes and restart, and do not modify text,
encoding, dirty flags or undo history. Old sessions keep their previous control
character visibility. The document map and search-results display stay uncluttered.

## Find in Files

Use **Search > Find in files...** (**Ctrl+Shift+F**). Enter **Folder:** or use
**Choose folder...**, then set **File filters:** such as
`*.rs;*.toml;!generated*`. `*` matches any filename characters, `?` matches one,
spaces/semicolons separate patterns, and `!` excludes filenames. Filters apply
to basenames, not directory paths, and are case-sensitive on Linux. Empty
filters include all filenames. **Include subfolders** defaults on;
**Include hidden files** defaults off.

The **Find in files** button reuses the query, normal/extended/regex mode, case
and whole-word options. Searches run in the background over disk contents,
not unsaved buffers, and use the bottom result panel and its cancellation and
navigation controls. A result verifies disk content before opening/selecting
the match. Changed files and differing unsaved buffers are rejected without
replacing the open text.

Binary, linked, oversized and unreadable files are skipped with visible warnings.
Limits are 128 MiB/file (including decoded UTF-8), 512 MiB total, 10,000 searched files, 100,000 visited
entries, 30 seconds between checks and 10,000 retained matches. A running
filesystem read or regex evaluation may delay cancellation. Folder-wide
replacement is not implemented.

## Find All and search results

Open Find/Replace with **Ctrl+F** or **Ctrl+H**, enter a query, and use
**Find all: current document** or **Find all: all open documents**. Both commands
are also in **Search**. Searches use the same normal/extended/regex,
case-sensitive and whole-word options as Find Next. Unsaved edits and unnamed
tabs are included; unopened files on disk are not searched.

The search form uses permanent mnemonic labels, framed entries and ordinary
Adwaita buttons rather than flat labels. Actions have native hover, pressed,
focus and disabled states in light/dark modes. The grid puts Find All on its
own row so controls stay separated at smaller window sizes; no custom CSS is
required.

The bottom panel groups matches by document, showing line/column and highlighted
snippets. Double-click or press **Enter** on a result to select the exact source
range. **F4 / Shift+F4** or **Next / Previous** wraps through valid results.
Document groups can be folded; drag the native divider to resize the panel.
**Close** or **Escape** while the results editor is focused hides it without
discarding results; **Ctrl+Alt+R** reopens it. **Clear** removes the list and
**Cancel** interrupts an active search. Results are read-only, but can be selected
and copied.

Searching uses background document snapshots and does not move the editing caret.
Changed/closed documents cannot be navigated using stale results: rerun Find All.
Unaffected documents remain navigable. Multiline and zero-width matches are
supported; snippets are bounded, but navigation selects the complete match.
Columns count Unicode characters and honor tab stops. Each search replaces the
previous list; results are transient and are not stored in recovery.

Find All retains **10,000 matches**, reporting truncation explicitly. Limits:
**128 MiB per document**, **256 MiB combined**. Match collection and replacement
phases allow two seconds per 16 MiB of input (up to 16 seconds); Find All allows
ten seconds per 64 MiB combined (up to 40 seconds). Budgets are checked between
matches, not hard execution deadlines. Cancellation
is checked between matches/documents; an in-progress regex evaluation may
finish first. Errors are reported, not silently converted to zero matches.

## Keyboard

| Shortcut | Action |
|---|---|
| Ctrl+N / Ctrl+O / Ctrl+S | New / open / save |
| Ctrl+Shift+S / Ctrl+W | Save as / close tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+F or Ctrl+H | Find and replace |
| F3 / Shift+F3 | Next / previous match |
| Ctrl+Alt+Enter / Ctrl+Shift+Enter | Find All in current document / all open documents |
| F4 / Shift+F4 | Next / previous Find All result |
| Ctrl+Alt+R | Toggle search-results panel |
| Ctrl+D / Ctrl+Shift+L | Select next / all occurrences |
| Ctrl+U / Ctrl+Shift+U | Lowercase / uppercase selections |
| Ctrl+Space / Ctrl+Shift+Space | Completion suggestions / function parameter hint |
| Ctrl+Alt+Right / F6 | Toggle split / focus other pane |
| F7 / Shift+F7 | Next / previous difference |
| Ctrl+Alt+J / Ctrl+Alt+T | Format JSON / toggle JSON tree |
| Ctrl+mouse wheel | Editor zoom |
| Escape | Close search bar, or hide search results when that panel is focused |

Extended search/replacement recognizes `\n`, `\r`, `\t`, `\\`, `\0`,
`\xHH` and `\uHHHH`. Invalid escapes are reported, not silently interpreted.
Regex uses the Rust `fancy-regex` engine, with multiline/CRLF-aware anchors and Unicode
word boundaries. Replacement captures use `$1` or `${name}`; `$$` is a
literal dollar. Look-around and pattern backreferences are supported, for
example `(?<=prefix:)(\w+)\s+\1(?!x)`. This is not complete Boost/PCRE dialect
compatibility. Backtracking is limited to 500,000 steps, bulk operations check
input-scaled processing budgets (two seconds per 16 MiB), and oversized expansions fail before replacing
the document. Regex errors are reported, not treated as "no match".

## Recovery and data safety

Recovery is stored as **plaintext** in `$XDG_STATE_HOME/rstpd-gtk/session.json`,
or `~/.local/state/rstpd-gtk/session.json` when `XDG_STATE_HOME` is unset.
New recovery directories are private to the user (0700), and new files use
0600 permissions. This may include sensitive unsaved text. Nothing is uploaded.
The Linux port does not inspect or migrate Windows recovery directories.
`--session-dir` explicitly selects a separate session, protected by an
advisory lock held for the process lifetime. The lock file is deliberately
retained after exit; an existing lock file alone does not indicate a running app.
Snapshots run approximately every three seconds after changes; a crash can
lose edits since the last completed snapshot. Normal exit waits for the final
snapshot and refuses to exit if that save fails.

Closing the app preserves every tab without asking for filenames.
Closing an individual dirty tab offers Save / Discard / Cancel; **Discard
removes that tab's recovery copy**. Autosave never writes the original files.
Explicit Save uses a flushed temporary file, same-directory atomic rename
and parent-directory synchronization. Existing file permission bits are retained.
An external-change check warns before overwriting a file that changed on disk.
It is a check, not an exclusive lock against other editors.

Linux saving requires working directory `fsync`, enforceable private permissions,
and `renameat2(RENAME_NOREPLACE)` support for new files. Unsupported operations
fail explicitly rather than using a non-atomic fallback. Some FAT/exFAT mount
configurations cannot satisfy the private-permission checks. Recovery directories
must be user-owned and not writable by other users; XDG/HOME roots must be absolute.

Existing hard-linked, special-mode, owner-read-only or foreign-owned files are
refused rather than silently changing their link/ownership semantics. Explicit
saves follow valid file symlinks without replacing the links; dangling links and
recovery-file symlinks are rejected. Ordinary permissions and owner/group are
preserved, **but ACLs and extended attributes are not preserved by atomic
replacement**. Avoid saving metadata-sensitive files with this initial port.

Invalid recovery files are left untouched and reported at startup. To recover
manually, keep a copy of the file, then move it out of the session directory.
On reopen, named tabs use their recovery snapshots; a changed disk version is
not silently substituted for the recovered text.

Without a Unicode BOM, valid UTF-8 is preferred; otherwise the editor opens as
Windows-1252 and reports the assumption (`.nfo` files use OEM 437 instead).
Use **Encoding > Reopen** to explicitly
reinterpret bytes. Reopen discards current edits only after confirmation.
Conversions that cannot represent every character are rejected.

## Deliberate boundaries

- Maximum document size: 256 MiB, including decoded UTF-8 text.
- Find/replace and directory search: 128 MiB per document/file; Find All: 256 MiB combined.
- Line operations and JSON tools: 16 MiB. Compare: 16 MiB combined.
- Search/replacement expressions: 32 KiB; capture expansion is size-bounded.
- Recovery file: 512 MiB; at most 256 tabs. Failed backups are visible in the status bar.
- Larger files require additional memory for decoding, editor storage, undo and search/recovery snapshots; limits are not a memory-availability guarantee. Older releases may reject recovery files above their former 256 MiB limit.
- JSON trees: 20,000 nodes and 128 nesting levels. Duplicate
  object keys are rejected to prevent silent data loss. Number spellings and
  object key order are preserved. JSON and JSON5 are supported.
  Tree pointers are limited to 64 KiB each and tree metadata to 8 MiB total.
  Formatting is independent of the tree-node limit, with a 500,000-token bound.
- Compare includes inline differences but is not a merge tool. A processing
  budget and 32 KiB inline-line limit may produce coarser highlighting.
- Completion is bounded local analysis plus API data, not language-server
  semantic completion. At most 10,000 imported overloads are retained.
- Language imports are limited to 1 MiB each and 64 installed definitions.
  UDL 1.x, arbitrary lexer libraries, executable plugin APIs and UI theme packs
  are unsupported. Custom highlighting has a nesting/time budget.
- This is an initial independent release, not complete Notepad++ feature parity.
  No printing, macro recording, installer, code signing or update service.

## Build from source

Install Rust 1.95 or newer, a C++17 compiler, `pkg-config`, `unzip`, and GTK 3.24+
development libraries. GTK remains a system-provided runtime dependency;
Scintilla and Lexilla are statically linked.

Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libgtk-3-dev unzip
```

Arch Linux:

```sh
sudo pacman -S --needed base-devel gtk3 unzip rust
```

Build and run:

```sh
./scripts/bootstrap.sh
cargo build --locked --release
./target/release/rstpd
```

The bootstrap script verifies pinned SHA-256 hashes before extracting the
vendored Scintilla/Lexilla sources and SciTE language data. Rust dependencies
are locked in `Cargo.lock`; the first build needs access to the Rust package
registry. No sources are downloaded by the native build.

For development and validation:

```sh
cargo run --locked
cargo test --locked --tests
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

GUI tests need a display. On a headless machine, install `xvfb` (Debian/Ubuntu)
or `xorg-server-xvfb` (Arch) and run:

```sh
xvfb-run -a cargo test --locked --tests
```

`./scripts/build.sh` combines bootstrap, tests and release build. The GTK
workflow test uses isolated temporary recovery data, covering command wiring,
split/map views, search and replacement, comparison refresh, JSON navigation,
encoding/EOL, imported definitions, completion and recovery. Native editor
tests also exercise all lexers, Unicode/NUL handling, multiple selections,
rectangular editing, undo, and visible Markdown styles in both palettes.
They also cover persisted symbol preferences, Unicode/selection character
counts, native search-control affordances/layout, read-only result navigation,
cancellation and stale/closed-document behavior. Manually dispatched CI runs
upload a verified Linux build artifact without creating a GitHub Release.

For a per-user launcher, put `target/release/rstpd` on your `PATH` and install
`assets/io.github.Sockolet.rstpd-gtk.desktop` under
`~/.local/share/applications/`. No installer, Flatpak or distribution package
is included. Keep the license and third-party notices with redistributed builds.

## Source layout and trust boundary

- `src/ui.rs`: GTK3 controls, event queue, background work and command wiring.
- `src/ui/tests.rs`: GTK workflow and recovery regression tests.
- `src/toolbar.rs`: standard GTK symbolic toolbar buttons and tooltips.
- `src/editor.rs`: owning GTK editor adapter, typed Scintilla messages and view configuration.
- `src/native_bridge.cxx`: native document release and notification adapter.
- `src/completion.rs`: contextual suggestions, declarations, signatures and API data import.
- `src/udl.rs`: non-executable language definitions and Rust container highlighting.
- `src/markdown.rs`: Markdown source spans, semantic styles and folding.
- `src/syntax.rs`: shared semantic token roles and font-attribute selection.
- `src/json_tools.rs`: bounded JSON/JSON5 validation and lossless formatting.
- `src/core.rs`: portable encoding, bounded search, text transforms, diff and JSON spans.
- `src/search_results.rs`: bounded multi-document matches, Unicode previews and result-row mappings.
- `src/symbols.rs`: persisted display flags and non-printing character names.
- `src/session.rs`: atomic persistence, Linux advisory locking and recovery worker.
- `src/languages.rs`: app-owned language aliases and keyword selection.
- `build.rs`: static native builds and build-time extraction of lexer constants,
  keyword data and semantic style roles.

There is no runtime lexer DLL loading or plugin path. The build reads only
language keyword/style data from SciTE; none of SciTE's commands or application
code are compiled into rstpd. JSON nodes are plain text, not rendered HTML.
Document contents do not launch programs or fetch resources.

Native document references have explicit ownership and automatic release,
including after their views are destroyed. Editor views are UI-thread-bound.
Scalar messages are checked against the pinned interface; pointer-bearing
messages require an explicit unsafe call behind typed buffer wrappers, and
text ranges are checked for bounds and UTF-8 boundaries.

Removing plugins eliminates that extension surface; it does **not** make an
editor vulnerability-free. Scintilla/Lexilla are native C++ dependencies and
remain part of the trust boundary. Dependency updates require rebuilding and
redistributing the application. This project is not affiliated with Notepad++.
