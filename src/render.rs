use syntect::{
    easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet,
    util::as_24_bit_terminal_escaped,
};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";

pub fn render_markdown(input: &str) -> String {
    let mut output = String::new();
    let mut lines = input.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(separator) = lines.peek().copied() {
            if is_table_header(line) && is_table_separator(separator) {
                let alignments = parse_alignments(lines.next().unwrap());
                let mut rows = vec![split_table_row(line)];
                while let Some(next) = lines.peek().copied() {
                    if !is_table_row(next) {
                        break;
                    }
                    rows.push(split_table_row(lines.next().unwrap()));
                }
                output.push_str(&render_table(rows, &alignments));
                continue;
            }
        }
        if let Some((fence, language)) = line
            .strip_prefix("```")
            .map(|language| ("```", language))
            .or_else(|| line.strip_prefix("~~~").map(|language| ("~~~", language)))
        {
            let language = language.trim();
            let mut code = String::new();
            for next in lines.by_ref() {
                if next.trim() == fence {
                    break;
                }
                code.push_str(next);
                code.push('\n');
            }
            output.push_str(&highlight_code(&code, language));
            continue;
        }
        output.push_str(&render_line(line));
        output.push('\n');
    }
    output.trim_end_matches('\n').to_owned()
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

fn terminal_width() -> usize {
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

fn highlight_code(code: &str, language: &str) -> String {
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let theme_set = ThemeSet::load_defaults();
    let theme = theme_set
        .themes
        .get("base16-ocean.dark")
        .or_else(|| theme_set.themes.values().next());
    let Some(theme) = theme else {
        return format!("{DIM}{code}{RESET}");
    };
    let syntax = syntax_set
        .find_syntax_by_token(language)
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut output = format!("{DIM}┌─ {language}\n{RESET}");
    for line in code.lines() {
        let highlighted = highlighter
            .highlight_line(line, &syntax_set)
            .unwrap_or_default();
        output.push_str("│ ");
        output.push_str(&as_24_bit_terminal_escaped(&highlighted[..], false));
        output.push('\n');
    }
    output.push_str(&format!("{DIM}└─{RESET}\n"));
    output
}

#[cfg(test)]
mod tests {
    use super::render_markdown;

    #[test]
    fn renders_table_and_code_block() {
        let rendered = render_markdown(
            "| Name | Value |\n| --- | --- |\n| hi | `ok` |\n\n```rust\nfn main() {}\n```",
        );
        assert!(rendered.contains("┌"));
        assert!(rendered.contains("hi"));
        assert!(rendered.contains("fn"));
        assert!(rendered.contains("main"));
        assert!(rendered.contains("\x1b["));
    }

    #[test]
    fn renders_extended_markdown() {
        let rendered = render_markdown(
            "1. First\n   - Nested\n\\*literal asterisk\\*\n\n| Left | Center | Right |\n| :--- | :---: | ---: |\n| a | b | c |\n\n~~~python\nprint('hi')\n~~~",
        );
        assert!(rendered.contains("1."));
        assert!(rendered.contains("Nested"));
        assert!(rendered.contains("*literal asterisk*"));
        assert!(rendered.contains("Center:"));
        assert!(rendered.contains("print"));
    }
}
