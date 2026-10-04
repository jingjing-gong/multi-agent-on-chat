//! Keep strong emphasis separated from adjacent words in Lark's renderer.
//! Do not rewrite code, escaped delimiters, links, or incomplete streamed pairs.
pub fn normalize(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut fence: Option<(char, usize)> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let first = trimmed.chars().next().unwrap_or_default();
        let count = trimmed.chars().take_while(|c| *c == first).count();
        if (first == char::from(96) || first == '~') && count >= 3 {
            match fence {
                None => fence = Some((first, count)),
                Some((mark, size))
                    if mark == first && count >= size && trimmed[count..].trim().is_empty() =>
                {
                    fence = None
                }
                _ => {}
            }
            result.push_str(line);
        } else if fence.is_some() {
            result.push_str(line);
        } else {
            result.push_str(&normalize_line(line));
        }
    }
    result
}
fn normalize_line(line: &str) -> String {
    let mut protected = vec![false; line.len()];
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let end = if i + 1 < bytes.len() {
                i + 1 + line[i + 1..].chars().next().unwrap().len_utf8()
            } else {
                i + 1
            };
            protected[i..end].fill(true);
            i = end;
        } else if bytes[i] == 96 {
            let n = bytes[i..].iter().take_while(|b| **b == 96).count();
            let delimiter = char::from(96).to_string().repeat(n);
            let end = line[i + n..]
                .find(&delimiter)
                .map(|p| i + n + p + n)
                .unwrap_or(line.len());
            protected[i..end].fill(true);
            i = end;
        } else if bytes[i] == b'[' || bytes[i] == b'<' {
            // Leave links, image labels and HTML tags intact.
            let close = if bytes[i] == b'[' { ']' } else { '>' };
            let mut end = line[i..]
                .find(close)
                .map(|p| i + p + 1)
                .unwrap_or(line.len());
            if line[end..].starts_with('(') {
                end = line[end..]
                    .find(')')
                    .map(|p| end + p + 1)
                    .unwrap_or(line.len());
            }
            protected[i..end].fill(true);
            i = end;
        } else {
            i += line[i..].chars().next().unwrap().len_utf8();
        }
    }
    let markers: Vec<usize> = (0..bytes.len().saturating_sub(1))
        .filter(|&p| {
            bytes[p] == b'*'
                && bytes[p + 1] == b'*'
                && !protected[p]
                && !protected[p + 1]
                && (p == 0 || bytes[p - 1] != b'*')
                && (p + 2 == bytes.len() || bytes[p + 2] != b'*')
        })
        .collect();
    let mut insert = std::collections::BTreeSet::new();
    for pair in markers.as_chunks::<2>().0 {
        let (open, close) = (pair[0], pair[1]);
        let body = &line[open + 2..close];
        if body.is_empty()
            || body.starts_with(char::is_whitespace)
            || body.ends_with(char::is_whitespace)
        {
            continue;
        }
        if line[..open]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
        {
            insert.insert(open);
        }
        if line[close + 2..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric)
        {
            insert.insert(close + 2);
        }
    }
    let mut out = String::new();
    for (p, c) in line.char_indices() {
        if insert.contains(&p) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strong_boundaries_and_literal_content() {
        assert_eq!(
            normalize("- **Agent 层：**决定\n一张**系统地图**，"),
            "- **Agent 层：** 决定\n一张 **系统地图**，"
        );
        let ticks = char::from(96).to_string();
        let literal=format!("{}bash\n**输出：**正文\n{}\n{}**code：**text{}\n[**label：**text](url)\n\\*\\*literal\\*\\*\n**未结束", ticks.repeat(4),ticks.repeat(4),ticks,ticks);
        assert_eq!(normalize(&literal), literal);
        let text = "**Agent 层：**决定";
        for (i, _) in text.char_indices() {
            let _ = normalize(&text[..i]);
        }
        assert_eq!(normalize(&normalize(text)), normalize(text));
    }
}
