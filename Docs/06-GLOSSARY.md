# Glossary

Terms used across the other documents in this set, defined once here rather
than re-explained inline each time.

**PTY (pseudoterminal)** — an OS-provided pair of virtual devices (a
"master" and a "slave" side) that lets a program (the terminal) talk to a
shell or other process as if it were a real physical terminal. The terminal
holds the master side; the shell is connected to the slave side. This is the
same OS primitive `tmux`, `screen`, and every terminal emulator use.

**VT / ANSI escape sequences** — special byte sequences embedded in
otherwise-plain-text output that instruct the terminal to do something other
than print a character: move the cursor, change color, switch to the
alternate screen, etc. "VT" refers to the historical VT100/VT220 terminal
hardware whose sequences became the de facto standard; "ANSI" refers to the
overlapping standardization of many of the same sequences.

**Grid** — the in-memory model of what's currently on screen: a 2D array of
cells, each holding a character plus its styling (foreground/background
color, bold/italic/underline, etc.) and cursor position/visibility state.

**Scrollback** — the history of previously-on-screen lines that have
scrolled off the visible grid but are still retrievable (e.g. by scrolling
up or searching).

**Alternate screen (alt-screen)** — a second, separate grid that full-screen
programs (`vim`, `less`, `tmux`, `htop`) switch the terminal into so their
UI doesn't get mixed into your normal scrollback, and which is discarded
(restoring the original grid/scrollback) when the program exits.

**Damage tracking** — only redrawing the regions of the screen that actually
changed since the last frame, rather than re-rendering the entire grid every
frame. Improves throughput under heavy output; does not, by itself, improve
input latency (see `01-ARCHITECTURE.md` § 5).

**Glyph atlas** — a single GPU texture containing pre-rasterized images of
every glyph (character shape) currently in use, so rendering a frame means
drawing textured quads referencing this atlas rather than re-rasterizing
text from font outlines every frame.

**Present mode** — the GPU/windowing-system policy for how a rendered frame
is handed off to the display (e.g. immediately vs. synchronized to the
display's refresh rate). Different present modes trade off latency against
tearing/smoothness; this project prioritizes the lowest-latency option (see
`01-ARCHITECTURE.md` § 5).

**Text shaping** — converting a sequence of Unicode characters into the
actual sequence of glyphs and their positions to draw, accounting for
ligatures, combining characters, and complex scripts where character-to-glyph
isn't a simple 1:1 mapping.

**Client / server (in this project's specific sense)** — the server process
owns PTYs and terminal state and is headless; the client process owns the
window, rendering, and input, and talks to the server over a socket or SSH
session. See `04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md`.

**Workspace / tab / pane (in this project's specific sense)** — a server
manages workspaces; each workspace contains tabs; each tab contains one or
more panes (created via splits); each pane owns exactly one PTY. See
`04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` § 5.

**Agent state (idle / working / blocked / done)** — this project's
classification of what a pane is currently doing, inferred by watching PTY
output, used to drive the notification/sidebar UX and the external socket
API for coding agents. See `04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` § 7.
