//! Sanitized Markdown preview.
//!
//! Equivalent to marked + DOMPurify for the cases the preview tests cover:
//! headings, emphasis, GFM tables, fenced code, disabled task checkboxes,
//! and a sanitizer that strips script, event handlers, javascript: URLs,
//! and unexpected tags. HTML is not executed.

#![allow(dead_code)]

const ALLOWED_TAGS: &[&str] = &[
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "code",
    "table",
    "thead",
    "tbody",
    "tr",
    "th",
    "td",
    "a",
    "img",
    "em",
    "strong",
    "del",
    "hr",
    "br",
    "input",
    "span",
];

const ALLOWED_ATTR: &[&str] = &[
    "href", "src", "alt", "title", "class", "type", "checked", "disabled", "align", "colspan",
    "rowspan", "id",
];

fn slugify(text: &str) -> String {
    let lower = text.trim().to_lowercase();
    let mut cleaned = String::new();
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c.is_whitespace() || c == '-' {
            cleaned.push(c);
        }
    }
    let mut out = String::new();
    let mut prev_hyphen = false;
    for c in cleaned.chars() {
        if c.is_whitespace() || c == '_' || c == '-' {
            if !prev_hyphen && !out.is_empty() {
                out.push('-');
                prev_hyphen = true;
            }
        } else {
            out.push(c);
            prev_hyphen = false;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "heading".to_string()
    } else {
        out
    }
}

fn escape_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            _ => out.push(c),
        }
    }
    out
}

fn inline(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = String::new();
    while i < chars.len() {
        if chars[i] == '`' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '`' {
                j += 1;
            }
            if j < chars.len() {
                let code: String = chars[i + 1..j].iter().collect();
                out.push_str("<code>");
                out.push_str(&escape_text(&code));
                out.push_str("</code>");
                i = j + 1;
                continue;
            }
        }
        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_close(&chars, i + 2, &['*', '*']) {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str("<strong>");
                out.push_str(&inline(&inner));
                out.push_str("</strong>");
                i = end + 2;
                continue;
            }
        }
        if i + 1 < chars.len() && chars[i] == '~' && chars[i + 1] == '~' {
            if let Some(end) = find_close(&chars, i + 2, &['~', '~']) {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str("<del>");
                out.push_str(&inline(&inner));
                out.push_str("</del>");
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if let Some((alt, url, next)) = parse_link(&chars, i + 1) {
                out.push_str("<img src=\"");
                out.push_str(&escape_attr(&url));
                out.push_str("\" alt=\"");
                out.push_str(&escape_attr(&alt));
                out.push_str("\">");
                i = next;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some((text, url, next)) = parse_link(&chars, i) {
                out.push_str("<a href=\"");
                out.push_str(&escape_attr(&url));
                out.push_str("\">");
                out.push_str(&inline(&text));
                out.push_str("</a>");
                i = next;
                continue;
            }
        }
        if chars[i] == '*' {
            if let Some(end) = find_single(&chars, i + 1, '*') {
                let inner: String = chars[i + 1..end].iter().collect();
                out.push_str("<em>");
                out.push_str(&inline(&inner));
                out.push_str("</em>");
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '<' {
            let rest: String = chars[i..].iter().collect();
            let (raw, next_rel) = take_raw_html(&rest);
            if next_rel > 1 {
                out.push_str(&raw);
                i += next_rel;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn find_close(chars: &[char], from: usize, marker: &[char]) -> Option<usize> {
    let mut i = from;
    while i + marker.len() <= chars.len() {
        if chars[i..i + marker.len()] == *marker {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_single(chars: &[char], from: usize, marker: char) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == marker && (i + 1 >= chars.len() || chars[i + 1] != marker) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_link(chars: &[char], open_bracket: usize) -> Option<(String, String, usize)> {
    if chars.get(open_bracket) != Some(&'[') {
        return None;
    }
    let mut i = open_bracket + 1;
    let start = i;
    while i < chars.len() && chars[i] != ']' {
        i += 1;
    }
    if i >= chars.len() || i + 1 >= chars.len() || chars[i + 1] != '(' {
        return None;
    }
    let text: String = chars[start..i].iter().collect();
    let mut j = i + 2;
    let url_start = j;
    while j < chars.len() && chars[j] != ')' {
        j += 1;
    }
    if j >= chars.len() {
        return None;
    }
    let url: String = chars[url_start..j].iter().collect();
    Some((text, url, j + 1))
}

fn take_raw_html(src: &str) -> (String, usize) {
    let chars: Vec<char> = src.chars().collect();
    if chars.first() != Some(&'<') {
        return (String::new(), 0);
    }
    if chars.get(1) == Some(&'/') {
        let mut i = 2;
        while i < chars.len() && chars[i] != '>' {
            i += 1;
        }
        if i < chars.len() {
            let raw: String = chars[..=i].iter().collect();
            return (raw, i + 1);
        }
        return (String::new(), 0);
    }
    let mut i = 1;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c == '>' {
            let raw: String = chars[..=i].iter().collect();
            return (raw, i + 1);
        }
        i += 1;
    }
    (String::new(), 0)
}

fn is_heading(line: &str) -> bool {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|c| *c == '#').count();
    hashes >= 1 && hashes <= 6 && t.chars().nth(hashes) == Some(' ')
}

fn heading_parts(line: &str) -> Option<(usize, String)> {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = t[hashes..].trim_start();
    if rest.is_empty() && !t[hashes..].starts_with(' ') && !t[hashes..].starts_with('\t') {
        return None;
    }
    if t.len() == hashes {
        return None;
    }
    if !t[hashes..].starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    Some((hashes, rest.to_string()))
}

fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.matches('|').count() >= 2
}

fn is_table_sep(line: &str) -> bool {
    if !is_table_row(line) {
        return false;
    }
    split_cells(line).iter().all(|c| {
        let c = c.trim();
        let c = c.trim_matches(':');
        !c.is_empty() && c.chars().all(|ch| ch == '-')
    })
}

fn split_cells(line: &str) -> Vec<String> {
    let t = line.trim().trim_matches('|');
    t.split('|').map(|c| c.trim().to_string()).collect()
}

fn is_list_item(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        return true;
    }
    let mut chars = t.chars().peekable();
    let mut saw_digit = false;
    while let Some(c) = chars.peek().copied() {
        if c.is_ascii_digit() {
            saw_digit = true;
            chars.next();
        } else {
            break;
        }
    }
    saw_digit && chars.next() == Some('.') && chars.next() == Some(' ')
}

fn list_body(line: &str) -> String {
    let t = line.trim_start();
    if let Some(rest) = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .or_else(|| t.strip_prefix("+ "))
    {
        return rest.to_string();
    }
    if let Some(idx) = t.find(". ") {
        return t[idx + 2..].to_string();
    }
    t.to_string()
}

fn is_block_start(line: &str) -> bool {
    is_heading(line) || is_fence(line) || is_list_item(line) || is_table_row(line)
}

fn markdown_to_html(src: &str) -> String {
    let lines: Vec<&str> = if src.is_empty() {
        Vec::new()
    } else {
        src.split('\n').collect()
    };
    let mut i = 0;
    let mut out = String::new();
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        if is_fence(line) {
            let lang = line.trim_start().trim_start_matches('`').trim();
            i += 1;
            let mut body = String::new();
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(lines[i]);
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            out.push_str("<pre><code");
            if !lang.is_empty() {
                out.push_str(" class=\"language-");
                out.push_str(&escape_attr(lang));
                out.push('"');
            }
            out.push('>');
            out.push_str(&escape_text(&body));
            out.push_str("</code></pre>\n");
            continue;
        }
        if let Some((depth, text)) = heading_parts(line) {
            let id = slugify(&text);
            out.push_str(&format!("<h{depth} id=\"{id}\">"));
            out.push_str(&inline(&text));
            out.push_str(&format!("</h{depth}>\n"));
            i += 1;
            continue;
        }
        if i + 1 < lines.len() && is_table_row(line) && is_table_sep(lines[i + 1]) {
            let header = split_cells(line);
            i += 2;
            let mut body_rows = Vec::new();
            while i < lines.len() && is_table_row(lines[i]) && !is_table_sep(lines[i]) {
                body_rows.push(split_cells(lines[i]));
                i += 1;
            }
            out.push_str("<table>\n<thead>\n<tr>\n");
            for cell in &header {
                out.push_str("<th>");
                out.push_str(&inline(cell));
                out.push_str("</th>\n");
            }
            out.push_str("</tr>\n</thead>\n<tbody>\n");
            for row in body_rows {
                out.push_str("<tr>\n");
                for cell in row {
                    out.push_str("<td>");
                    out.push_str(&inline(&cell));
                    out.push_str("</td>\n");
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</tbody>\n</table>\n");
            continue;
        }
        if is_list_item(line) {
            let ordered = line
                .trim_start()
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
            let tag = if ordered { "ol" } else { "ul" };
            out.push_str(&format!("<{tag}>\n"));
            while i < lines.len() && is_list_item(lines[i]) {
                let body = list_body(lines[i]);
                out.push_str("<li>");
                if let Some(rest) = body.strip_prefix("[ ] ") {
                    out.push_str("<input disabled=\"\" type=\"checkbox\"> ");
                    out.push_str(&inline(rest));
                } else if let Some(rest) = body
                    .strip_prefix("[x] ")
                    .or_else(|| body.strip_prefix("[X] "))
                {
                    out.push_str("<input checked=\"\" disabled=\"\" type=\"checkbox\"> ");
                    out.push_str(&inline(rest));
                } else if body == "[ ]" || body == "[x]" || body == "[X]" {
                    let checked = body != "[ ]";
                    if checked {
                        out.push_str("<input checked=\"\" disabled=\"\" type=\"checkbox\">");
                    } else {
                        out.push_str("<input disabled=\"\" type=\"checkbox\">");
                    }
                } else {
                    out.push_str(&inline(&body));
                }
                out.push_str("</li>\n");
                i += 1;
            }
            out.push_str(&format!("</{tag}>\n"));
            continue;
        }
        let mut para = vec![line.to_string()];
        i += 1;
        while i < lines.len() && !lines[i].trim().is_empty() && !is_block_start(lines[i]) {
            para.push(lines[i].to_string());
            i += 1;
        }
        let text = para.join(" ");
        out.push_str("<p>");
        out.push_str(&inline(&text));
        out.push_str("</p>\n");
    }
    out
}

fn safe_href(value: &str) -> bool {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    lower.starts_with("https:")
        || lower.starts_with("http:")
        || lower.starts_with("mailto:")
        || lower.starts_with('#')
}

fn safe_src(value: &str) -> bool {
    value.trim().to_ascii_lowercase().starts_with("data:image/")
}

fn allowed_tag(name: &str) -> bool {
    ALLOWED_TAGS.contains(&name)
}

fn sanitize(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut out = String::new();
    while i < chars.len() {
        if chars[i] != '<' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if chars.get(i + 1) == Some(&'/') {
            let (name, next) = parse_end_tag(&chars, i);
            if let Some(name) = name {
                if allowed_tag(&name) {
                    out.push_str("</");
                    out.push_str(&name);
                    out.push('>');
                }
                i = next;
                continue;
            }
            out.push('<');
            i += 1;
            continue;
        }
        let parsed = parse_start_tag(&chars, i);
        let Some((name, attrs, _self_close, next)) = parsed else {
            out.push('<');
            i += 1;
            continue;
        };
        let lname = name.to_lowercase();
        if lname == "script" || lname == "style" {
            i = skip_until_close(&chars, next, &lname);
            continue;
        }
        if !allowed_tag(&lname) {
            i = next;
            continue;
        }
        out.push('<');
        out.push_str(&lname);
        if lname == "input" {
            let checked = attrs
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("checked") && v.as_deref() != Some("false"));
            if checked {
                out.push_str(" checked=\"\"");
            }
            out.push_str(" disabled=\"\" type=\"checkbox\"");
        } else {
            for (k, v) in &attrs {
                let key = k.to_lowercase();
                if key.starts_with("on") {
                    continue;
                }
                if !ALLOWED_ATTR.contains(&key.as_str()) {
                    continue;
                }
                if key == "href" {
                    let value = v.clone().unwrap_or_default();
                    if !safe_href(&value) {
                        continue;
                    }
                    out.push(' ');
                    out.push_str(&key);
                    out.push_str("=\"");
                    out.push_str(&escape_attr(value.trim()));
                    out.push('"');
                    continue;
                }
                if key == "src" {
                    let value = v.clone().unwrap_or_default();
                    if !safe_src(&value) {
                        continue;
                    }
                    out.push(' ');
                    out.push_str(&key);
                    out.push_str("=\"");
                    out.push_str(&escape_attr(value.trim()));
                    out.push('"');
                    continue;
                }
                if key == "id" {
                    let value = v.clone().unwrap_or_default();
                    let heading = matches!(lname.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
                    let slug = value
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                        && !value.is_empty();
                    if !heading || !slug {
                        continue;
                    }
                    out.push_str(" id=\"");
                    out.push_str(&escape_attr(&value));
                    out.push('"');
                    continue;
                }
                if let Some(value) = v {
                    out.push(' ');
                    out.push_str(&key);
                    out.push_str("=\"");
                    out.push_str(&escape_attr(value));
                    out.push('"');
                } else {
                    out.push(' ');
                    out.push_str(&key);
                    out.push_str("=\"\"");
                }
            }
        }
        out.push('>');
        i = next;
    }
    out
}

fn parse_end_tag(chars: &[char], i: usize) -> (Option<String>, usize) {
    if chars.get(i) != Some(&'<') || chars.get(i + 1) != Some(&'/') {
        return (None, i);
    }
    let mut j = i + 2;
    let start = j;
    while j < chars.len() && (chars[j].is_ascii_alphanumeric()) {
        j += 1;
    }
    if j == start {
        return (None, i);
    }
    let name: String = chars[start..j].iter().collect::<String>().to_lowercase();
    while j < chars.len() && chars[j] != '>' {
        j += 1;
    }
    if j >= chars.len() {
        return (None, i);
    }
    (Some(name), j + 1)
}

fn parse_start_tag(
    chars: &[char],
    i: usize,
) -> Option<(String, Vec<(String, Option<String>)>, bool, usize)> {
    if chars.get(i) != Some(&'<') {
        return None;
    }
    let mut j = i + 1;
    if j < chars.len() && chars[j] == '/' {
        return None;
    }
    let start = j;
    while j < chars.len() && chars[j].is_ascii_alphanumeric() {
        j += 1;
    }
    if j == start {
        return None;
    }
    let name: String = chars[start..j].iter().collect();
    let mut attrs = Vec::new();
    loop {
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        if j >= chars.len() {
            return None;
        }
        if chars[j] == '>' {
            return Some((name, attrs, false, j + 1));
        }
        if chars[j] == '/' && chars.get(j + 1) == Some(&'>') {
            return Some((name, attrs, true, j + 2));
        }
        let name_start = j;
        while j < chars.len()
            && (chars[j].is_ascii_alphanumeric() || chars[j] == '-' || chars[j] == ':')
        {
            j += 1;
        }
        if j == name_start {
            return None;
        }
        let attr: String = chars[name_start..j].iter().collect();
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        if j < chars.len() && chars[j] == '=' {
            j += 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j >= chars.len() {
                return None;
            }
            if chars[j] == '"' || chars[j] == '\'' {
                let q = chars[j];
                j += 1;
                let vstart = j;
                while j < chars.len() && chars[j] != q {
                    j += 1;
                }
                let value: String = chars[vstart..j].iter().collect();
                if j < chars.len() {
                    j += 1;
                }
                attrs.push((attr, Some(value)));
            } else {
                let vstart = j;
                while j < chars.len() && !chars[j].is_whitespace() && chars[j] != '>' {
                    j += 1;
                }
                let value: String = chars[vstart..j].iter().collect();
                attrs.push((attr, Some(value)));
            }
        } else {
            attrs.push((attr, None));
        }
    }
}

fn skip_until_close(chars: &[char], from: usize, name: &str) -> usize {
    let close = format!("</{name}");
    let close_chars: Vec<char> = close.chars().collect();
    let mut i = from;
    while i + close_chars.len() <= chars.len() {
        if chars[i..i + close_chars.len()]
            .iter()
            .zip(close_chars.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
        {
            let mut j = i + close_chars.len();
            while j < chars.len() && chars[j] != '>' {
                j += 1;
            }
            if j < chars.len() {
                return j + 1;
            }
            return chars.len();
        }
        i += 1;
    }
    chars.len()
}

pub fn render_markdown_to_safe_html(source: &str) -> String {
    sanitize(&markdown_to_html(source))
}

pub fn is_markdown_name(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    if !name.contains('.') {
        return false;
    }
    matches!(ext.as_str(), "md" | "markdown" | "mdown" | "mkd")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownPreviewMode {
    Edit,
    Side,
    Preview,
}

/// Cycle Edit → Side → Preview → Edit for Markdown files.
pub fn next_markdown_preview_mode(cur: Option<MarkdownPreviewMode>) -> MarkdownPreviewMode {
    match cur {
        Some(MarkdownPreviewMode::Side) => MarkdownPreviewMode::Preview,
        Some(MarkdownPreviewMode::Preview) => MarkdownPreviewMode::Edit,
        _ => MarkdownPreviewMode::Side,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_headings_with_deterministic_ids() {
        let html = render_markdown_to_safe_html("# Hello World!");
        assert!(
            html.contains("<h1 id=\"hello-world\">Hello World!</h1>"),
            "{html}"
        );
    }

    #[test]
    fn renders_emphasis_and_strikethrough() {
        let html = render_markdown_to_safe_html("**bold** *em* ~~strike~~");
        assert!(html.contains("<strong>bold</strong>"), "{html}");
        assert!(html.contains("<em>em</em>"), "{html}");
        assert!(html.contains("<del>strike</del>"), "{html}");
    }

    #[test]
    fn renders_gfm_tables() {
        let html = render_markdown_to_safe_html("| a | b |\n|---|---|\n| 1 | 2 |");
        assert!(html.contains("<table>"), "{html}");
        assert!(html.contains("<thead>"), "{html}");
        assert!(html.contains("<tbody>"), "{html}");
        assert!(html.contains("<th"), "{html}");
        assert!(html.contains("<td"), "{html}");
    }

    #[test]
    fn renders_fenced_code_with_language_class() {
        let html = render_markdown_to_safe_html("```js\nconst x = 1;\n```");
        assert!(html.contains("<pre>"), "{html}");
        assert!(html.contains("<code class=\"language-js\">"), "{html}");
    }

    #[test]
    fn renders_task_list_checkboxes_as_disabled() {
        let html = render_markdown_to_safe_html("- [ ] todo\n- [x] done");
        assert!(html.contains("<input"), "{html}");
        assert!(html.contains("type=\"checkbox\""), "{html}");
        assert!(
            html.contains("disabled=\"\"") || html.contains("disabled"),
            "{html}"
        );
        assert!(
            html.contains("checked=\"\"") || html.contains("checked"),
            "{html}"
        );
        assert!(!html.contains("enabled"), "{html}");
    }

    #[test]
    fn removes_script_tags_and_inline_event_handlers() {
        let html =
            render_markdown_to_safe_html("<script>alert(1)</script>\n<img src=x onerror=alert(1)>");
        assert!(!html.contains("<script>"), "{html}");
        assert!(!html.contains("onerror"), "{html}");
        assert!(!html.contains("alert(1)"), "{html}");
    }

    #[test]
    fn neutralizes_javascript_links() {
        let html = render_markdown_to_safe_html("[x](javascript:alert(1))");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(!html.contains("alert(1)"), "{html}");
    }

    #[test]
    fn allows_data_images_and_blocks_remote() {
        let data_url = "data:image/png;base64,iVBORw0KGgo=";
        let html = render_markdown_to_safe_html(&format!(
            "![ok]({data_url})\n![remote](https://example.com/x.png)\n![rel](./x.png)"
        ));
        assert!(html.contains(&format!("src=\"{data_url}\"")), "{html}");
        assert!(!html.contains("https://example.com"), "{html}");
        assert!(!html.contains("./x.png"), "{html}");
    }

    #[test]
    fn preserves_safe_external_links() {
        let html = render_markdown_to_safe_html("[link](https://example.com)");
        assert!(html.contains("href=\"https://example.com\""), "{html}");
        assert!(html.contains("<a"), "{html}");
    }

    #[test]
    fn preserves_hash_links() {
        let html = render_markdown_to_safe_html("[section](#section-one)");
        assert!(html.contains("href=\"#section-one\""), "{html}");
    }

    #[test]
    fn removes_unexpected_tags() {
        let html = render_markdown_to_safe_html("<iframe src=\"evil\"></iframe>");
        assert!(!html.contains("<iframe"), "{html}");
    }

    #[test]
    fn cycles_edit_side_preview() {
        assert_eq!(next_markdown_preview_mode(None), MarkdownPreviewMode::Side);
        assert_eq!(
            next_markdown_preview_mode(Some(MarkdownPreviewMode::Edit)),
            MarkdownPreviewMode::Side
        );
        assert_eq!(
            next_markdown_preview_mode(Some(MarkdownPreviewMode::Side)),
            MarkdownPreviewMode::Preview
        );
        assert_eq!(
            next_markdown_preview_mode(Some(MarkdownPreviewMode::Preview)),
            MarkdownPreviewMode::Edit
        );
    }
}
