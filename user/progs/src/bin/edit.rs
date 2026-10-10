//! `edit FILE` -- a nano-style full-screen editor. Typing and deleting, save (`^S`) and save-as
//! (`^O`), the header bar, the message/prompt row, the key hints, exiting with unsaved changes, and the `^G` help
//! screen; line cut/copy/paste (`^K`/`M-6`/`^U`), forward search (`^W`, `M-W` for find next)
//! and go to line (`^_`/`M-G`); `M-A` sets or clears a mark, after which `^K`/`M-6`/`^U`
//! act on the *region* between it and the cursor (character-granular, drawn `ATTR_INVERSE`)
//! instead of the whole line; `M-N` toggles a line-number gutter; `M-I` toggles auto-indent; `M-O` makes the
//! Tab key type spaces up to the next tab stop instead of a tab character.
//! (`M-X` is nano's notation for Alt+X, and is what the key hints and help screen use.)
//!
//! Row 0 is the inverse-video header (name, `[Modified]`, position); rows 1..R-4 are the text
//! area; row R-3 is a transient message or the one-line prompt widget, blank otherwise; the last
//! two rows are the key hints (`MENU`), nano-style.
//!
//! Reads keys with `CONSOLE_READ_KEY` and draws whole frames with `CONSOLE_DRAW` (see
//! `abi::ioctl`); nothing here is a raw-mode toggle on `read(0)`; the two are independent readers
//! of the same token queue (`keyboard/stdin.rs`'s doc comment). Exiting restores whatever was on
//! screen before this program's first frame (the console's "alternate screen").
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use abi::errno::{ENOENT, errmsg};
use abi::ioctl::{ATTR_DIM, ATTR_INVERSE, CELL_SIZE, CONSOLE_DRAW_HEADER_SIZE, Cell, ConsoleDraw};
use abi::keys::{
    KEY_6, KEY_A, KEY_BACKSLASH, KEY_BACKSPACE, KEY_C, KEY_DELETE, KEY_DOWN, KEY_END, KEY_ENTER,
    KEY_ESC, KEY_G, KEY_HOME, KEY_I, KEY_K, KEY_LEFT, KEY_MINUS, KEY_N, KEY_O, KEY_PAGEDOWN,
    KEY_PAGEUP, KEY_RIGHT, KEY_S, KEY_SLASH, KEY_TAB, KEY_U, KEY_UP, KEY_W, KEY_X,
};
use editlib::buffer::Buffer;
use editlib::editrc::{self, Config};
use editlib::layout::{self, Row};
use editlib::prompt::Prompt;
use editlib::region;
use editlib::render::{self, render_row};
use editlib::scroll::{self, Top};
use editlib::search;
use progs::cli::{extra_operand, missing_operand, report, report_errno};
use progs::{read_fd, write_all_fd};
use userlib::{
    Args, Env, ExitCode, O_RDONLY, O_WRONLY, close, console_draw, open, read_key, winsize,
};

userlib::entry_with_env!(run);

const USAGE: &str = "edit FILE";
const FLAGS: &[(&str, &str)] = &[];

/// Rows outside the text area: the header above it, the message/prompt row and the two key-hint
/// rows below.
const RESERVED_ROWS: usize = 4;

/// The screen's shape: `TIOCGWINSZ`'s rows and columns, the configured tab stop width, and whether
/// the line-number gutter is on (`M-N` toggles it, so unlike the other three this one *can*
/// change) -- the inputs `draw` needs alongside the buffer and overlay state, bundled together only
/// so `draw` doesn't take eight separate arguments.
#[derive(Clone, Copy)]
struct Screen {
    rows: usize,
    cols: usize,
    tab_size: usize,
    line_numbers: bool,
}

impl Screen {
    /// Rows left for the text area once the header, the message/prompt row and the key hints are
    /// set aside -- at least 1, even for a degenerate `rows`.
    fn text_height(&self) -> usize {
        self.rows.saturating_sub(RESERVED_ROWS).max(1)
    }
}

/// Digits in `n`, at least 1 (so `0`, which `line_count` and `line + 1` never actually are, would
/// still get a sensible answer rather than none).
fn digits(mut n: usize) -> usize {
    let mut count = 1;
    while n >= 10 {
        n /= 10;
        count += 1;
    }
    count
}

/// The line-number gutter's width: digits of the highest line number plus one separator column, or
/// `0` when the gutter is off. A free function, not a `Screen` method, since it also needs
/// `buffer.line_count()` -- which changes as lines are added or removed, so this is recomputed fresh
/// each frame rather than cached anywhere, exactly the rule ("the line count gaining a
/// digit... simply changes it and the next frame re-lays-out").
fn gutter_width(line_numbers: bool, line_count: usize) -> usize {
    if line_numbers {
        digits(line_count) + 1
    } else {
        0
    }
}

/// Columns left for line content once the gutter (if any) is set aside -- what every wrapping,
/// scrolling and cursor-position computation uses in place of the screen's raw `cols`.
fn text_width(screen: Screen, buffer: &Buffer) -> usize {
    screen
        .cols
        .saturating_sub(gutter_width(screen.line_numbers, buffer.line_count()))
}

/// What the bottom row (and, for `Help`, the whole screen) currently shows instead of plain editing.
enum Overlay {
    /// Plain editing: the message row shows a transient `message` if one is set.
    None,
    /// A one-line question -- Save As, search, or go to line -- with `Enter` to confirm and `Esc`
    /// to cancel.
    Prompt(Prompt, PromptPurpose),
    /// "Save modified buffer? (Y/N/Cancel)" -- `^X` on a modified buffer.
    Confirm,
    /// The full-screen key list `^G` shows; any key returns to `None`.
    Help,
}

enum PromptPurpose {
    SaveAs,
    /// `^W`: search forward for the typed term from the cursor, wrapping around once.
    Search,
    /// `^_`/`M-G`: jump to the typed `LINE` or `LINE,COL` (both 1-based).
    Goto,
}

fn run(args: Args, env: Env) -> ExitCode {
    let plain = match progs::cli::plain("edit", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    let Some(path) = plain.first else {
        return missing_operand("edit");
    };
    if plain.count > 1
        && let Some(extra) = plain.second
    {
        return extra_operand("edit", extra);
    }

    let (config, mut message) = load_editrc(env);

    let mut filename = path.to_string();
    let mut buffer = match load_file(path) {
        Ok(buffer) => buffer,
        Err(code) => {
            report_errno("edit", path, code);
            return ExitCode(1);
        }
    };

    let Ok(size) = winsize(1) else {
        report("edit", "standard output is not a terminal");
        return ExitCode(1);
    };
    let (rows, cols) = (usize::from(size.rows), usize::from(size.cols));
    let tab_size = usize::from(config.tab_size);
    let mut auto_indent = config.auto_indent;
    let mut tabs_to_spaces = config.tabs_to_spaces;

    let mut top = Top::default();
    let mut preferred_col: Option<usize> = None;
    let mut overlay = Overlay::None;
    let mut cut_buffer = String::new();
    let mut cut_streak = false; // whether the *previous* key was `^K`, so this one extends it
    let mut cut_is_region = false; // whether `cut_buffer` is a region (paste_region) or lines (paste_line)
    let mut last_search: Option<String> = None;
    let mut mark: Option<(usize, usize)> = None; // `M-A`'s mark -- the region is (mark, cursor)

    let mut screen = Screen {
        rows,
        cols,
        tab_size,
        line_numbers: config.line_numbers,
    };
    let text_height = screen.text_height();

    loop {
        let width = text_width(screen, &buffer);
        top = scroll::scroll_to_cursor(&buffer, top, text_height, width, tab_size);
        draw(
            &buffer,
            top,
            screen,
            &filename,
            &overlay,
            message.as_deref(),
            mark,
        );

        let Ok(event) = read_key(0) else {
            return ExitCode(1); // stdin isn't the keyboard -- nothing this program can do about it
        };
        let code = event.effective_code();
        message = None; // a message is shown for exactly one frame, then cleared on the next key
        let mut is_cut_key = false; // set only by the `^K` arm below, to extend `cut_streak`

        match &mut overlay {
            Overlay::Help => {
                overlay = Overlay::None;
            }

            Overlay::Confirm => match event.char() {
                Some('y' | 'Y') => match try_save(&buffer, &filename) {
                    Ok(_) => break,
                    Err(e) => {
                        message = Some(format!("Error writing {filename}: {}", errmsg(e)));
                        overlay = Overlay::None;
                    }
                },
                Some('n' | 'N') => break,
                _ if code == KEY_ESC => overlay = Overlay::None,
                _ => {}
            },

            Overlay::Prompt(prompt, purpose) => match code {
                KEY_ESC => overlay = Overlay::None,
                KEY_ENTER => {
                    let answer = prompt.input().to_string();
                    match purpose {
                        PromptPurpose::SaveAs => {
                            if answer.is_empty() {
                                overlay = Overlay::None;
                            } else {
                                match try_save(&buffer, &answer) {
                                    Ok(lines) => {
                                        filename = answer;
                                        buffer.mark_saved();
                                        message = Some(format!("Wrote {lines} lines"));
                                    }
                                    Err(e) => {
                                        message =
                                            Some(format!("Error writing {answer}: {}", errmsg(e)));
                                    }
                                }
                                overlay = Overlay::None;
                            }
                        }
                        PromptPurpose::Search => {
                            overlay = Overlay::None;
                            if !answer.is_empty() {
                                message = do_search(&mut buffer, &mut top, screen, &answer);
                                last_search = Some(answer);
                            }
                        }
                        PromptPurpose::Goto => {
                            overlay = Overlay::None;
                            match search::parse_goto(&answer, &buffer) {
                                Some((line, byte)) => {
                                    buffer.set_cursor(line, byte);
                                    top = scroll::center_on(
                                        &buffer,
                                        line,
                                        screen.text_height(),
                                        width,
                                        tab_size,
                                    );
                                }
                                None if !answer.is_empty() => {
                                    message = Some("Not a line number".to_string());
                                }
                                None => {}
                            }
                        }
                    }
                }
                KEY_LEFT => {
                    prompt.move_left();
                }
                KEY_RIGHT => {
                    prompt.move_right();
                }
                KEY_HOME => {
                    prompt.move_home();
                }
                KEY_END => {
                    prompt.move_end();
                }
                KEY_BACKSPACE => {
                    prompt.backspace();
                }
                KEY_DELETE => {
                    prompt.delete_forward();
                }
                _ if !event.ctrl() && !event.alt() => {
                    if let Some(c) = event.char()
                        && !c.is_control()
                    {
                        prompt.insert_char(c);
                    }
                }
                _ => {}
            },

            Overlay::None => {
                if code != KEY_UP && code != KEY_DOWN {
                    preferred_col = None;
                }
                match code {
                    // Word movement, before the plain-arrow arms below, which would otherwise also
                    // match a Ctrl-held one (`match` takes the first arm that fits the value alone;
                    // the guard is what makes this one win only when Ctrl is actually held).
                    KEY_LEFT if event.ctrl() => {
                        buffer.move_word_left();
                    }
                    KEY_RIGHT if event.ctrl() => {
                        buffer.move_word_right();
                    }
                    KEY_LEFT => {
                        buffer.move_left();
                    }
                    KEY_RIGHT => {
                        buffer.move_right();
                    }
                    KEY_UP => {
                        scroll::move_vertical(
                            &mut buffer,
                            true,
                            &mut preferred_col,
                            width,
                            tab_size,
                        );
                    }
                    KEY_DOWN => {
                        scroll::move_vertical(
                            &mut buffer,
                            false,
                            &mut preferred_col,
                            width,
                            tab_size,
                        );
                    }
                    KEY_HOME => {
                        buffer.move_home();
                    }
                    KEY_END => {
                        buffer.move_end();
                    }
                    KEY_PAGEUP => page(
                        &mut buffer,
                        &mut preferred_col,
                        true,
                        text_height,
                        width,
                        tab_size,
                    ),
                    KEY_PAGEDOWN => page(
                        &mut buffer,
                        &mut preferred_col,
                        false,
                        text_height,
                        width,
                        tab_size,
                    ),
                    KEY_BACKSLASH if event.alt() => {
                        buffer.move_to_first_line();
                    }
                    KEY_SLASH if event.alt() => {
                        buffer.move_to_last_line();
                    }
                    KEY_BACKSPACE => {
                        mark = None; // a stale mark could no longer name a valid region
                        buffer.backspace();
                    }
                    KEY_DELETE => {
                        mark = None;
                        buffer.delete_forward();
                    }
                    KEY_ENTER => {
                        mark = None;
                        buffer.split_line(auto_indent);
                    }
                    KEY_S if event.ctrl() && !event.repeat() => {
                        match try_save(&buffer, &filename) {
                            Ok(lines) => {
                                buffer.mark_saved();
                                message = Some(format!("Wrote {lines} lines"));
                            }
                            Err(e) => {
                                message = Some(format!("Error writing {filename}: {}", errmsg(e)));
                            }
                        }
                    }
                    KEY_O if event.ctrl() && !event.repeat() => {
                        overlay = Overlay::Prompt(
                            Prompt::new("File Name to Write: ", &filename),
                            PromptPurpose::SaveAs,
                        );
                    }
                    KEY_G if event.ctrl() && !event.repeat() => {
                        overlay = Overlay::Help;
                    }
                    KEY_C if event.ctrl() && !event.repeat() => {
                        message = Some(position_message(&buffer));
                    }
                    KEY_K if event.ctrl() && !event.repeat() => {
                        if let Some(m) = mark {
                            let (start, end) = region::ordered(m, buffer.cursor());
                            cut_buffer = region::cut_region(&mut buffer, start, end);
                            cut_is_region = true;
                            mark = None; // the region it named is gone
                        } else {
                            is_cut_key = true;
                            let text = region::cut_line(&mut buffer);
                            cut_buffer = if cut_streak {
                                format!("{cut_buffer}\n{text}")
                            } else {
                                text
                            };
                            cut_is_region = false;
                        }
                    }
                    KEY_6 if event.alt() => {
                        if let Some(m) = mark {
                            let (start, end) = region::ordered(m, buffer.cursor());
                            cut_buffer = region::region_text(&buffer, start, end);
                            cut_is_region = true;
                            mark = None; // copied, not cut, but the selection is now spent
                        } else {
                            let (line, _) = buffer.cursor();
                            cut_buffer = buffer.line(line).to_string();
                            cut_is_region = false;
                        }
                    }
                    KEY_U if event.ctrl() && !event.repeat() => {
                        mark = None;
                        if cut_is_region {
                            region::paste_region(&mut buffer, &cut_buffer);
                        } else {
                            region::paste_line(&mut buffer, &cut_buffer);
                        }
                    }
                    KEY_A if event.alt() => {
                        mark = if mark.is_some() {
                            None
                        } else {
                            Some(buffer.cursor())
                        };
                        message = Some(
                            if mark.is_some() {
                                "Mark Set"
                            } else {
                                "Mark Unset"
                            }
                            .to_string(),
                        );
                    }
                    KEY_N if event.alt() => {
                        screen.line_numbers = !screen.line_numbers;
                    }
                    KEY_O if event.alt() => {
                        tabs_to_spaces = !tabs_to_spaces;
                        message = Some(
                            if tabs_to_spaces {
                                "Typed tabs converted to spaces"
                            } else {
                                "Typed tabs kept as tabs"
                            }
                            .to_string(),
                        );
                    }
                    KEY_TAB if tabs_to_spaces && !event.ctrl() && !event.alt() => {
                        mark = None;
                        let (line, byte) = buffer.cursor();
                        let fill = layout::tab_fill(buffer.line(line), byte, width, tab_size);
                        for _ in 0..fill {
                            buffer.insert_char(' ');
                        }
                    }
                    KEY_I if event.alt() => {
                        auto_indent = !auto_indent;
                        message = Some(
                            if auto_indent {
                                "Auto indent enabled"
                            } else {
                                "Auto indent disabled"
                            }
                            .to_string(),
                        );
                    }
                    KEY_W if event.ctrl() && !event.repeat() => {
                        overlay = Overlay::Prompt(
                            Prompt::new("Search: ", last_search.as_deref().unwrap_or("")),
                            PromptPurpose::Search,
                        );
                    }
                    KEY_W if event.alt() => match &last_search {
                        Some(term) => {
                            message = do_search(&mut buffer, &mut top, screen, term);
                        }
                        None => message = Some("No previous search".to_string()),
                    },
                    KEY_MINUS if event.ctrl() && event.shift() => {
                        overlay =
                            Overlay::Prompt(Prompt::new("Go To Line: ", ""), PromptPurpose::Goto);
                    }
                    KEY_G if event.alt() => {
                        overlay =
                            Overlay::Prompt(Prompt::new("Go To Line: ", ""), PromptPurpose::Goto);
                    }
                    KEY_X if event.ctrl() && !event.repeat() => {
                        if buffer.is_modified() {
                            overlay = Overlay::Confirm;
                        } else {
                            break;
                        }
                    }
                    _ if !event.ctrl() && !event.alt() => {
                        if let Some(c) = event.char()
                            && (!c.is_control() || c == '\t')
                        {
                            mark = None;
                            buffer.insert_char(c);
                        }
                    }
                    _ => {}
                }
            }
        }
        cut_streak = is_cut_key; // any key but `^K` itself ends a run of accumulating cuts
    }
    ExitCode(0)
}

/// Searches for `term` from the cursor (`search::find`'s own rule: inclusive of where the cursor
/// already is, so a fresh search may match at once, but `M-W`'s repeat never re-finds the same
/// occurrence, because a successful search leaves the cursor at the match's *end*). Re-centers the
/// view on a match, the same as a go-to-line jump -- both are "the cursor could now be anywhere",
/// unlike ordinary movement, which `scroll_to_cursor` handles incrementally. Returns `"Not found"` as
/// a message on failure, `None` (nothing to show) on success.
fn do_search(buffer: &mut Buffer, top: &mut Top, screen: Screen, term: &str) -> Option<String> {
    let from = buffer.cursor();
    match search::find(buffer, from, term) {
        Some((line, byte)) => {
            buffer.set_cursor(line, byte + term.len());
            *top = scroll::center_on(
                buffer,
                line,
                screen.text_height(),
                text_width(screen, buffer),
                screen.tab_size,
            );
            None
        }
        None => Some("Not found".to_string()),
    }
}

/// Moves the cursor a screenful of rows up or down, one row at a time (so it still lands correctly
/// inside a line taller than the screen, or crosses several short ones) -- `^Y`/`^V`.
fn page(
    buffer: &mut Buffer,
    preferred_col: &mut Option<usize>,
    up: bool,
    rows: usize,
    width: usize,
    tab_size: usize,
) {
    for _ in 0..rows {
        if !scroll::move_vertical(buffer, up, preferred_col, width, tab_size) {
            break; // already at the very first or last row
        }
    }
}

/// Truncates `path` and writes the whole buffer to it (in place, no temp file). Returns the number of lines written, or the negative error from whichever syscall
/// failed first -- `open`, the write itself, or `close` (a full disk can surface there instead).
fn try_save(buffer: &Buffer, path: &str) -> Result<usize, isize> {
    let fd = open(path, O_WRONLY);
    if fd < 0 {
        return Err(fd);
    }
    let fd = fd as usize;
    let text = buffer.to_text();
    let write_result = write_all_fd(fd, text.as_bytes());
    let close_result = close(fd);
    write_result?;
    if close_result < 0 {
        return Err(close_result);
    }
    Ok(buffer.line_count())
}

/// Reads `$HOME/.editrc`, if `$HOME` is set and the file exists. The first problem it reports, if
/// any, comes back as a message for the message row's very first frame; the rest are silently
/// applied (their own setting keeps its default) but not individually shown -- one line is what the
/// row has room for.
fn load_editrc(env: Env) -> (Config, Option<String>) {
    let Some(home) = env
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(key, value)| (key == "HOME").then_some(value))
    else {
        return (Config::default(), None);
    };
    let mut path = String::from(home);
    path.push_str("/.editrc");

    let fd = open(&path, O_RDONLY);
    if fd < 0 {
        return (Config::default(), None); // missing (or unreadable): silent, every default applies
    }
    let bytes = read_fd(fd as usize);
    close(fd as usize);
    let text = String::from_utf8_lossy(&bytes);
    let parsed = editrc::parse(&text);
    let message = parsed.problems.first().map(|first| {
        let more = parsed.problems.len() - 1;
        let suffix = if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        };
        format!("{path}:{}: {}{suffix}", first.line, first.why)
    });
    (parsed.config, message)
}

/// Opens `path` and reads it whole, as LF-only text (lossy, not a validity check -- carriage
/// returns are dropped and invalid UTF-8 replaced, exactly as `String::from_utf8_lossy` already
/// does for the latter). A missing file is `Ok`, a fresh empty buffer -- everything else is
/// the syscall's own error.
fn load_file(path: &str) -> Result<Buffer, isize> {
    let fd = open(path, O_RDONLY);
    if fd == ENOENT {
        return Ok(Buffer::new());
    }
    if fd < 0 {
        return Err(fd);
    }
    let bytes = read_fd(fd as usize);
    close(fd as usize);
    let text = String::from_utf8_lossy(&bytes).replace('\r', "");
    Ok(Buffer::from_text(&text))
}

/// `^C`'s message: the cursor's line and column (both 1-based; column counts characters, not display
/// columns, so a line with tabs or wide glyphs still gets a plain count).
fn position_message(buffer: &Buffer) -> String {
    let (line, byte) = buffer.cursor();
    let column = buffer.line(line)[..byte].chars().count() + 1;
    format!(
        "line {} of {}, column {column}",
        line + 1,
        buffer.line_count()
    )
}

/// The two key-hint rows under the message row, nano-style: each entry is a key (drawn inverse)
/// and what it does. Chosen to fit 80 columns; keys not listed (`M-\`/`M-/` for the first and
/// last line, movement, `^_` for go-to-line) are on the `^G` help screen.
const MENU: [&[(&str, &str)]; 2] = [
    &[
        ("^G", "Help"),
        ("^O", "Write"),
        ("^S", "Save"),
        ("^X", "Exit"),
        ("^C", "Pos"),
        ("^W", "Find"),
        ("M-W", "Next"),
        ("M-G", "Go To"),
    ],
    &[
        ("^K", "Cut"),
        ("M-6", "Copy"),
        ("^U", "Paste"),
        ("M-A", "Mark"),
        ("M-N", "Line Nos"),
        ("M-I", "Auto-indent"),
        ("M-O", "Tab>Spc"),
    ],
];

const HELP_TEXT: &[&str] = &[
    "edit -- A simple text editor",
    "",
    "Arrows, Home/End       move by character / logical line",
    "Ctrl+Left/Right        move by word",
    "PgUp/PgDn              move a screenful",
    "M-\\ / M-/            first / last line",
    "",
    "^S    Save",
    "^O    Write Out (save as)",
    "^X    Exit (asks first if modified)",
    "^C    Show the cursor's line and column",
    "^G    This help screen",
    "Esc   Cancel a prompt",
    "",
    "^K    Cut the line (repeat to cut several as one)",
    "M-6   Copy the line",
    "^U    Paste",
    "^W    Search forward (M-W: find the next match)",
    "^_ or M-G Go to a line, or line,column",
    "",
    "M-A   Set or clear the mark -- ^K/M-6/^U then act on the region",
    "M-N   Toggle the line-number gutter",
    "M-I   Toggle auto-indent",
    "M-O   Toggle typing spaces for the Tab key",
    "",
    "M- is the Alt key (as in nano)",
    "",
    "Press any key to continue",
];

/// Pushes `text` as `cols` cells, one column per character (plain ASCII status/footer/prompt text,
/// not the width-aware text area), all with `attr`, truncated or padded with blanks to fit exactly.
fn push_line(cells: &mut Vec<Cell>, cols: usize, text: &str, attr: u8) {
    let mut count = 0;
    for c in text.chars().take(cols) {
        cells.push(Cell { ch: c as u32, attr });
        count += 1;
    }
    for _ in count..cols {
        cells.push(Cell {
            ch: ' ' as u32,
            attr,
        });
    }
}

/// The header bar: `edit  name [Modified]   Ln x/y, Col z`, drawn inverse across the whole row.
fn render_header(cells: &mut Vec<Cell>, cols: usize, filename: &str, buffer: &Buffer) {
    let (line, byte) = buffer.cursor();
    let modified = if buffer.is_modified() {
        " [Modified]"
    } else {
        ""
    };
    let column = buffer.line(line)[..byte].chars().count() + 1;
    let text = format!(
        " edit  {filename}{modified}   Ln {}/{}, Col {column}",
        line + 1,
        buffer.line_count()
    );
    push_line(cells, cols, &text, ATTR_INVERSE);
}

/// The prompt row: `label` then the input typed so far, plain. Returns the column the cursor
/// belongs at (the label's width plus how far into the input it is).
fn render_prompt_row(cells: &mut Vec<Cell>, cols: usize, prompt: &Prompt) -> usize {
    let text = format!("{}{}", prompt.label(), prompt.input());
    push_line(cells, cols, &text, 0);
    prompt.label().chars().count() + prompt.input()[..prompt.cursor()].chars().count()
}

/// The message row when it isn't a prompt: a transient message, or (during `Overlay::Confirm`) the
/// save-modified question -- plain text, one line, blank when there is neither.
fn render_footer(cells: &mut Vec<Cell>, cols: usize, text: &str) {
    push_line(cells, cols, text, 0);
}

/// One row of key hints: each key inverse, its label plain, two blanks between entries, padded to
/// exactly `cols` cells.
fn render_menu_row(cells: &mut Vec<Cell>, cols: usize, entries: &[(&str, &str)]) {
    let start = cells.len();
    for (i, (key, label)) in entries.iter().enumerate() {
        if i > 0 {
            cells.extend([Cell::plain(' '); 2]);
        }
        cells.extend(key.chars().map(|c| Cell {
            ch: c as u32,
            attr: ATTR_INVERSE,
        }));
        cells.push(Cell::plain(' '));
        cells.extend(label.chars().map(Cell::plain));
    }
    cells.truncate(start + cols);
    cells.resize(start + cols, Cell::plain(' '));
}

/// The line-number gutter for one text-area row: `line`'s own number (1-based), right-aligned with
/// one trailing separator column, on the row that starts the line (`row.start == 0`); blank on a
/// continuation row of a line too long for one screen row. `ATTR_DIM` throughout, even the blank
/// cells, so a continuation row's gutter still reads as part of the gutter, not stray background.
/// Pushes exactly `width` cells; does nothing when `width` is `0` (the gutter off).
fn render_gutter(cells: &mut Vec<Cell>, width: usize, line: usize, row: Row) {
    if width == 0 {
        return;
    }
    if row.start == 0 {
        push_line(
            cells,
            width,
            &format!("{:>pad$} ", line + 1, pad = width - 1),
            ATTR_DIM,
        );
    } else {
        push_line(cells, width, "", ATTR_DIM);
    }
}

/// Draws the `^G` help screen: `HELP_TEXT`, one line per row, cursor parked at the top left (there
/// is nothing here to place it meaningfully on).
fn draw_help(rows: usize, cols: usize) {
    let mut cells = Vec::with_capacity(rows * cols);
    for row in 0..rows {
        let text = HELP_TEXT.get(row).copied().unwrap_or("");
        render_row(
            &mut cells,
            text,
            Row {
                start: 0,
                end: text.len(),
            },
            cols,
            8,
        );
    }
    send_frame(&cells, rows, cols, 0, 0);
}

/// Encodes `cells` behind a `ConsoleDraw` header and sends them in one `CONSOLE_DRAW` call.
fn send_frame(cells: &[Cell], rows: usize, cols: usize, cursor_row: u16, cursor_col: u16) {
    let header = ConsoleDraw {
        rows: rows as u16,
        cols: cols as u16,
        cursor_row,
        cursor_col,
    };
    let mut frame = Vec::with_capacity(CONSOLE_DRAW_HEADER_SIZE + cells.len() * CELL_SIZE);
    frame.extend_from_slice(&header.encode());
    for cell in cells {
        frame.extend_from_slice(&cell.encode());
    }
    let _ = console_draw(1, &frame);
}

/// Builds one whole frame and sends it. `Overlay::Help` replaces the entire screen; otherwise the
/// header, the text area (`rows - RESERVED_ROWS` rows), the message/prompt row and the key hints. The
/// cursor's screen row is found by matching its exact `Row` (from `layout::wrap_line`, which
/// `visible_rows` also built its list from) against that list, rather than re-deriving it a second
/// way that could disagree with what was actually drawn.
fn draw(
    buffer: &Buffer,
    top: Top,
    screen: Screen,
    filename: &str,
    overlay: &Overlay,
    message: Option<&str>,
    mark: Option<(usize, usize)>,
) {
    let Screen {
        rows,
        cols,
        tab_size,
        ..
    } = screen;
    if let Overlay::Help = overlay {
        draw_help(rows, cols);
        return;
    }

    let text_height = rows.saturating_sub(RESERVED_ROWS).max(1);
    let width = text_width(screen, buffer);
    let gutter = cols - width;
    let (cursor_line, cursor_byte) = buffer.cursor();
    let (cursor_screen_row, cursor_col) =
        layout::position_to_cell(buffer.line(cursor_line), cursor_byte, width, tab_size);
    let cursor_target: Row =
        layout::wrap_line(buffer.line(cursor_line), width, tab_size)[cursor_screen_row];
    let cursor_col = cursor_col + gutter; // the text area itself starts `gutter` columns in

    let region = mark.map(|m| region::ordered(m, buffer.cursor()));

    let mut cells = Vec::with_capacity(rows * cols);
    let mut cursor_row = 0;
    for (screen_row, (line, row)) in scroll::visible_rows(buffer, top, text_height, width, tab_size)
        .into_iter()
        .enumerate()
    {
        if line == cursor_line && row == cursor_target {
            cursor_row = screen_row;
        }
        render_gutter(&mut cells, gutter, line, row);
        let row_start = cells.len();
        render_row(&mut cells, buffer.line(line), row, width, tab_size);
        if let Some((start, end)) = region
            && let Some((from, to)) =
                render::region_columns(buffer.line(line), row, line, start, end, tab_size)
        {
            for cell in &mut cells[row_start + from..row_start + to] {
                cell.attr |= ATTR_INVERSE;
            }
        }
    }
    cells.resize(text_height * cols, Cell::plain(' ')); // past the end of the buffer: blank rows

    let (cursor_row, cursor_col, cursor_attr) = match overlay {
        Overlay::Prompt(prompt, _) => {
            let col = render_prompt_row(&mut cells, cols, prompt);
            (text_height, col, ATTR_INVERSE)
        }
        Overlay::Confirm => {
            render_footer(&mut cells, cols, "Save modified buffer? (Y/N/Cancel)");
            (cursor_row, cursor_col, ATTR_INVERSE)
        }
        Overlay::None => {
            render_footer(&mut cells, cols, message.unwrap_or(""));
            // With a mark set, the region's own highlight is plain `ATTR_INVERSE`; the cursor cell
            // sits one *past* the actual selected text (the region is [mark, cursor), never
            // including the cursor's own position) but would otherwise look identical to the cells
            // that genuinely are selected, since both use the same single attribute bit. Dimming it
            // too gives the cursor its own, distinct look (black on `DIM_FG`, `console/mod.rs`'s own
            // combination for `INVERSE | DIM`) so it reads as "the caret is here," not "one more
            // selected character."
            let attr = if mark.is_some() {
                ATTR_INVERSE | ATTR_DIM
            } else {
                ATTR_INVERSE
            };
            (cursor_row, cursor_col, attr)
        }
        Overlay::Help => unreachable!("handled above"),
    };

    // The frame is the header bar, then everything built above (text area, message/prompt row),
    // then the two key-hint rows; the cursor's row moves down by the header.
    let mut frame = Vec::with_capacity(rows * cols);
    render_header(&mut frame, cols, filename, buffer);
    frame.extend(cells);
    for entries in MENU {
        render_menu_row(&mut frame, cols, entries);
    }
    frame.resize(rows * cols, Cell::plain(' '));
    let cursor_row = cursor_row + 1;

    if let Some(cell) = frame.get_mut(cursor_row * cols + cursor_col) {
        cell.attr |= cursor_attr;
    }

    send_frame(&frame, rows, cols, cursor_row as u16, cursor_col as u16);
}
