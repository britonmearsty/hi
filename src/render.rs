use std::{
    io::{self, Write},
    sync::OnceLock,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::SyntaxSet,
    util::as_24_bit_terminal_escaped,
};
use unicode_width::UnicodeWidthChar;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}
fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}
fn code_theme() -> Option<&'static Theme> {
    theme_set()
        .themes
        .get("base16-ocean.dark")
        .or_else(|| theme_set().themes.values().next())
}

fn parse_alignments(line: &str) -> Vec<Alignment> {
    split_table_row(line)
        .into_iter()
        .map(|cell| {
            let cell = cell.trim();
            match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Alignment::Center,
                (false, true) => Alignment::Right,
                _ => Alignment::Left,
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Alignment {
    Left,
    Center,
    Right,
}

fn is_table_header(line: &str) -> bool {
    line.matches('|').count() >= 1 && !line.trim().is_empty()
}

fn is_table_row(line: &str) -> bool {
    line.matches('|').count() >= 1 && !line.trim().is_empty()
}

fn is_table_separator(line: &str) -> bool {
    let cells = split_table_row(line);
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let cell = cell.trim_matches(':').trim();
            cell.len() >= 3 && cell.chars().all(|character| character == '-')
        })
}

fn split_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

fn render_table(rows: Vec<Vec<String>>, alignments: &[Alignment]) -> String {
    let column_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    if column_count == 0 {
        return String::new();
    }
    let terminal_width = terminal_width();
    if terminal_width < 90 && column_count > 1 {
        return render_compact_table(&rows, terminal_width);
    }
    let mut widths = vec![6usize; column_count];
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(visible_length(cell));
        }
    }
    let available = terminal_width.saturating_sub(column_count + 1 + column_count * 2);
    while widths.iter().sum::<usize>() > available {
        let Some(index) = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > 6)
            .max_by_key(|(_, width)| **width)
            .map(|(index, _)| index)
        else {
            break;
        };
        widths[index] -= 1;
    }
    let border = |left: char, middle: char, right: char| {
        format!(
            "{left}{}{right}",
            widths
                .iter()
                .map(|width| "─".repeat(width + 2))
                .collect::<Vec<_>>()
                .join(&middle.to_string())
        )
    };
    let mut output = String::new();
    output.push_str(&format!("{DIM}{}{RESET}\n", border('┌', '┬', '┐')));
    for (row_index, row) in rows.iter().enumerate() {
        let wrapped: Vec<Vec<String>> = widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                wrap_text(row.get(index).map(String::as_str).unwrap_or(""), *width)
            })
            .collect();
        let line_count = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line_index in 0..line_count {
            output.push_str(&format!("{DIM}│{RESET}"));
            for (index, width) in widths.iter().enumerate() {
                let cell = wrapped[index]
                    .get(line_index)
                    .map(String::as_str)
                    .unwrap_or("");
                let rendered = render_inline(cell);
                let rendered = if row_index == 0 {
                    format!("{BOLD}{rendered}{RESET}")
                } else {
                    rendered
                };
                let padding = width.saturating_sub(visible_length(cell));
                let (left, right) = match alignments.get(index).copied().unwrap_or(Alignment::Left)
                {
                    Alignment::Left => (0, padding),
                    Alignment::Center => (padding / 2, padding - padding / 2),
                    Alignment::Right => (padding, 0),
                };
                output.push(' ');
                output.push_str(&" ".repeat(left));
                output.push_str(&rendered);
                output.push_str(&" ".repeat(right + 1));
                output.push_str(&format!("{DIM}│{RESET}"));
            }
            output.push('\n');
        }
        if row_index == 0 {
            output.push_str(&format!("{DIM}{}{RESET}\n", border('├', '┼', '┤')));
        }
    }
    output.push_str(&format!("{DIM}{}{RESET}\n", border('└', '┴', '┘')));
    output
}

fn render_compact_table(rows: &[Vec<String>], terminal_width: usize) -> String {
    let headers = &rows[0];
    let mut output = String::new();
    for (row_index, row) in rows.iter().skip(1).enumerate() {
        output.push_str(&format!("{BOLD}{CYAN}Row {}{RESET}\n", row_index + 1));
        for (index, header) in headers.iter().enumerate() {
            let value = row.get(index).map(String::as_str).unwrap_or("");
            let label = format!("{}: ", render_inline(header));
            let label_width = visible_length(header) + 2;
            let available = terminal_width.saturating_sub(label_width + 4).max(12);
            let wrapped = wrap_text(value, available);
            for (line_index, line) in wrapped.iter().enumerate() {
                if line_index == 0 {
                    output.push_str(&format!("  {label}{}\n", render_inline(line)));
                } else {
                    output.push_str(&format!(
                        "  {:width$}{}\n",
                        "",
                        render_inline(line),
                        width = label_width
                    ));
                }
            }
        }
        output.push('\n');
    }
    output
}

pub fn terminal_width() -> usize {
    terminal_size::terminal_size()
        .map(|(terminal_size::Width(width), _)| width as usize)
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse().ok())
        })
        .filter(|width: &usize| *width > 20)
        .unwrap_or(80)
}

fn wrap_text(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in value.split_whitespace() {
        if word.chars().count() > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let mut chunk = String::new();
            for character in word.chars() {
                chunk.push(character);
                if chunk.chars().count() == width {
                    lines.push(std::mem::take(&mut chunk));
                }
            }
            current = chunk;
        } else if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// Trim `rendered` to at most `max` terminal cells, appending a dim ellipsis
/// when content is cut. ANSI escape sequences are preserved and do not count
/// towards the width.
fn truncate_to_width(rendered: &str, max: usize) -> String {
    if StreamingRenderer::display_width(rendered) <= max {
        return rendered.to_owned();
    }
    let max = max.saturating_sub(1).max(1);
    let mut out = String::new();
    let mut width = 0;
    let mut truncated = false;
    let mut chars = rendered.chars();
    'outer: while let Some(character) = chars.next() {
        if character == '\x1b' {
            out.push(character);
            if chars.next() == Some('[') {
                out.push('[');
                for code in chars.by_ref() {
                    out.push(code);
                    if ('\x40'..='\x7e').contains(&code) {
                        continue 'outer;
                    }
                }
            }
            continue;
        }
        let cell = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + cell > max {
            truncated = true;
            break;
        }
        out.push(character);
        width += cell;
    }
    if truncated {
        out.push_str(RESET);
        out.push_str(DIM);
        out.push('…');
        out.push_str(RESET);
    }
    out
}

fn visible_length(value: &str) -> usize {
    value
        .chars()
        .filter(|character| !matches!(character, '*' | '_' | '`'))
        .count()
}

fn render_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let indentation = " ".repeat(line.len() - trimmed.len());
    if let Some(heading) = trimmed.strip_prefix("#") {
        let heading = heading.trim_start_matches('#').trim();
        return format!("{BOLD}{CYAN}{}{RESET}", render_inline(heading));
    }
    if let Some(item) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        return format!("{indentation}{CYAN}•{RESET} {}", render_inline(item));
    }
    if let Some((marker, item)) = ordered_item(trimmed) {
        return format!("{indentation}{CYAN}{marker}{RESET} {}", render_inline(item));
    }
    if let Some(quote) = trimmed.strip_prefix("> ") {
        return format!("{DIM}│{}{}", RESET, render_inline(quote.trim()));
    }
    if trimmed == "---" || trimmed == "***" {
        return format!("{DIM}────────────────────{RESET}");
    }
    render_inline(line)
}

fn ordered_item(line: &str) -> Option<(&str, &str)> {
    let marker_end = line.find(['.', ')'])?;
    if marker_end == 0
        || !line[..marker_end]
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return None;
    }
    let item = line.get(marker_end + 1..)?.strip_prefix(' ')?;
    Some((&line[..marker_end + 1], item))
}

fn render_inline(input: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            result.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_marker(&chars, i + 2, "**") {
                result.push_str(BOLD);
                result.extend(&chars[i + 2..end]);
                result.push_str(RESET);
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '*' || chars[i] == '_' {
            let marker = chars[i];
            if let Some(end) = chars[i + 1..]
                .iter()
                .position(|c| *c == marker)
                .map(|p| p + i + 1)
            {
                result.push_str(ITALIC);
                result.extend(&chars[i + 1..end]);
                result.push_str(RESET);
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '`' {
            if let Some(end) = chars[i + 1..]
                .iter()
                .position(|c| *c == '`')
                .map(|p| p + i + 1)
            {
                result.push_str(YELLOW);
                result.extend(&chars[i + 1..end]);
                result.push_str(RESET);
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some(close) = chars[i + 1..]
                .iter()
                .position(|c| *c == ']')
                .map(|p| p + i + 1)
            {
                if close + 1 < chars.len() && chars[close + 1] == '(' {
                    if let Some(end) = chars[close + 2..]
                        .iter()
                        .position(|c| *c == ')')
                        .map(|p| p + close + 2)
                    {
                        result.extend(&chars[i + 1..close]);
                        result.push(' ');
                        result.push_str(DIM);
                        result.push('(');
                        result.extend(&chars[close + 2..end]);
                        result.push(')');
                        result.push_str(RESET);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

fn find_marker(chars: &[char], start: usize, marker: &str) -> Option<usize> {
    let marker: Vec<char> = marker.chars().collect();
    chars[start..]
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|p| p + start)
}

struct CodeState {
    fence: &'static str,
    highlighter: Option<HighlightLines<'static>>,
}

struct TableState {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
    alignments: Vec<Alignment>,
}

/// Incrementally renders streamed Markdown into the terminal, buffering only
/// the minimum state needed for code blocks and pipe tables.
pub struct StreamingRenderer {
    buffer: String,
    output: String,
    code: Option<CodeState>,
    pending_header: Option<String>,
    table: Option<TableState>,
    partial_shown: bool,
    term_width: usize,
    finished: bool,
}

impl StreamingRenderer {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            output: String::new(),
            code: None,
            pending_header: None,
            table: None,
            partial_shown: false,
            term_width: terminal_width(),
            finished: false,
        }
    }

    /// Consume a streamed chunk, emitting every now-complete line.
    pub fn feed(&mut self, chunk: &str) {
        if self.finished {
            return;
        }
        self.buffer.push_str(chunk);
        while let Some(position) = self.buffer.find('\n') {
            let line = self.buffer[..position].to_owned();
            self.buffer.drain(..position + 1);
            self.handle_line(line.strip_suffix('\r').unwrap_or(&line));
        }
    }

    /// Render anything still buffered: a trailing partial line, an unfinished
    /// pipe table, or an unclosed code block.
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.handle_line(line.strip_suffix('\r').unwrap_or(&line));
        }
        if let Some(header) = self.pending_header.take() {
            self.emit(&format!("{}\n", render_line(&header)));
        }
        if let Some(table) = self.table.take() {
            let mut rows = vec![table.header];
            rows.extend(table.rows);
            self.emit(&render_table(rows, &table.alignments));
        }
        if self.code.is_some() {
            self.emit(&format!("{DIM}└─{RESET}\n"));
            self.code = None;
        }
    }

    /// Paint accumulated output and the in-flight partial line to stdout.
    ///
    /// Completed lines are printed once, top to bottom. The trailing partial
    /// line occupies exactly one terminal row: when it is wider than the
    /// terminal it is truncated with a dim ellipsis, so repainting it never
    /// wraps, scrolls the screen, or leaves ghost rows behind.
    pub fn paint(&mut self) {
        if self.partial_shown {
            print!("\r\x1b[2K");
        }
        if !self.output.is_empty() {
            print!("{}", self.output);
            self.output.clear();
        }
        if !self.buffer.is_empty() {
            let partial = truncate_to_width(&self.live_partial(), self.term_width);
            print!("\r\x1b[2K{}", partial);
            self.partial_shown = true;
        } else {
            self.partial_shown = false;
        }
        let _ = io::stdout().flush();
    }

    fn live_partial(&self) -> String {
        if self.code.is_some() {
            format!("│ {DIM}{}{RESET}", self.buffer)
        } else {
            render_inline(&self.buffer)
        }
    }

    /// Render a complete document to a styled string without writing to
    /// stdout. Used for one-shot, non-interactive output.
    pub fn render_once(&mut self, document: &str) -> String {
        self.feed(document);
        self.finish();
        std::mem::take(&mut self.output)
    }

    /// Rendered string width in terminal cells, ignoring ANSI escape codes.
    fn display_width(rendered: &str) -> usize {
        let mut width = 0;
        let mut chars = rendered.chars();
        while let Some(character) = chars.next() {
            if character == '\x1b' {
                if chars.next() == Some('[') {
                    for code in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&code) {
                            break;
                        }
                    }
                }
            } else {
                width += UnicodeWidthChar::width(character).unwrap_or(0);
            }
        }
        width
    }

    fn emit(&mut self, rendered: &str) {
        self.output.push_str(rendered);
    }

    fn handle_line(&mut self, line: &str) {
        if let Some(code) = self.code.as_mut() {
            if line.trim() == code.fence {
                self.emit(&format!("{DIM}└─{RESET}\n"));
                self.code = None;
            } else if let Some(highlighter) = code.highlighter.as_mut() {
                let highlighted = highlighter
                    .highlight_line(line, syntax_set())
                    .unwrap_or_default();
                self.emit(&format!(
                    "│ {}\n",
                    as_24_bit_terminal_escaped(&highlighted[..], false)
                ));
            } else {
                self.emit(&format!("│ {DIM}{line}{RESET}\n"));
            }
            return;
        }

        if self.pending_header.is_some() {
            let header = self.pending_header.take().unwrap();
            if is_table_separator(line) {
                self.table = Some(TableState {
                    header: split_table_row(&header),
                    rows: Vec::new(),
                    alignments: parse_alignments(line),
                });
                return;
            }
            self.emit(&format!("{}\n", render_line(&header)));
        }

        if let Some(table) = self.table.as_mut() {
            if is_table_row(line) {
                table.rows.push(split_table_row(line));
                return;
            }
            let finished = self.table.take().unwrap();
            let mut rows = vec![finished.header];
            rows.extend(finished.rows);
            self.emit(&render_table(rows, &finished.alignments));
        }

        if let Some((fence, language)) = line
            .strip_prefix("```")
            .map(|language| ("```", language))
            .or_else(|| line.strip_prefix("~~~").map(|language| ("~~~", language)))
        {
            let language = language.trim();
            self.emit(&format!("{DIM}┌─ {language}\n{RESET}"));
            let highlighter = code_theme().and_then(|theme| {
                syntax_set()
                    .find_syntax_by_token(language)
                    .map(|syntax| HighlightLines::new(syntax, theme))
            });
            self.code = Some(CodeState { fence, highlighter });
            return;
        }

        if is_table_header(line) {
            self.pending_header = Some(line.to_owned());
            return;
        }

        self.emit(&format!("{}\n", render_line(line)));
    }
}

#[cfg(test)]
mod tests {
    use super::{truncate_to_width, StreamingRenderer, BOLD, DIM, RESET};

    #[test]
    fn renders_table_and_code_block() {
        let mut renderer = StreamingRenderer::new();
        renderer
            .feed("| Name | Value |\n| --- | --- |\n| hi | `ok` |\n\n```rust\nfn main() {}\n```");
        renderer.finish();
        assert!(renderer.output.contains("┌"));
        assert!(renderer.output.contains("hi"));
        assert!(renderer.output.contains("fn"));
        assert!(renderer.output.contains("main"));
        assert!(renderer.output.contains("\x1b["));
    }

    #[test]
    fn renders_extended_markdown() {
        let mut renderer = StreamingRenderer::new();
        renderer.feed(
            "1. First\n   - Nested\n\\*literal asterisk\\*\n\n| Left | Center | Right |\n| :--- | :---: | ---: |\n| a | b | c |\n\n~~~python\nprint('hi')\n~~~",
        );
        renderer.finish();
        assert!(renderer.output.contains("1."));
        assert!(renderer.output.contains("Nested"));
        assert!(renderer.output.contains("*literal asterisk*"));
        assert!(renderer.output.contains("Center:"));
        assert!(renderer.output.contains("print"));
    }

    #[test]
    fn streams_plain_lines_and_keeps_partial_line_buffered() {
        let mut renderer = StreamingRenderer::new();
        renderer.feed("hello **world**");
        assert!(renderer.output.is_empty());
        renderer.feed(" and `code`\nnext line\n");
        assert!(renderer.output.contains("hello"));
        assert!(renderer.output.contains("world"));
        assert!(renderer.output.contains("\x1b["));
        renderer.finish();
        assert!(renderer.output.ends_with("line\n"));
    }

    #[test]
    fn streams_code_blocks_line_by_line() {
        let mut renderer = StreamingRenderer::new();
        renderer.feed("```rust\n");
        assert!(renderer.output.contains("┌─"));
        renderer.feed("fn main() {\n");
        assert!(renderer.output.contains("│ "));
        renderer.feed("}\n```\n");
        assert!(renderer.output.contains("└─"));
        renderer.finish();
    }

    #[test]
    fn buffers_tables_until_rows_are_done() {
        let mut renderer = StreamingRenderer::new();
        renderer.feed("| Name | Value |\n");
        assert!(renderer.output.is_empty());
        renderer.feed("| --- | --- |\n");
        assert!(renderer.output.is_empty());
        renderer.feed("| hi | `wow` |\n");
        assert!(renderer.output.is_empty());
        renderer.feed("after\n");
        assert!(renderer.output.contains("Name"));
        assert!(renderer.output.contains("wow"));
        assert!(renderer.output.contains("after"));
        renderer.finish();
    }

    #[test]
    fn closes_unfinished_constructs_on_finish() {
        let mut renderer = StreamingRenderer::new();
        renderer.feed("| Left | Right |\n| :--- | ---: |\n| a | b |\n```sh\n");
        renderer.feed("echo hi");
        renderer.finish();
        assert!(renderer.output.contains("┌"));
        assert!(renderer.output.contains("echo"));
        assert!(renderer.output.contains("└─"));
    }

    #[test]
    fn measures_display_width_ignoring_ansi() {
        assert_eq!(
            StreamingRenderer::display_width(&format!("{BOLD}hi{RESET}")),
            2
        );
        assert_eq!(StreamingRenderer::display_width("héllo"), 5);
        assert_eq!(StreamingRenderer::display_width("w寬"), 3);
    }

    #[test]
    fn truncates_partial_to_width() {
        assert_eq!(truncate_to_width("abc", 5), "abc");
        let cut = truncate_to_width("abcdefgh", 5);
        assert!(cut.starts_with("abcd"));
        assert!(cut.contains('…'));
        assert_eq!(
            truncate_to_width(&format!("{BOLD}abcdef{RESET}"), 5),
            format!("{BOLD}abcd{RESET}{DIM}…{RESET}")
        );
    }
}
