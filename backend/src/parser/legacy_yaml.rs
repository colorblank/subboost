use std::borrow::Cow;

#[derive(Default)]
struct FlowState {
    indents: Vec<usize>,
    quote: Option<u8>,
    node_start: bool,
    previous: u8,
    block_indent: Option<usize>,
    plain_indent: Option<usize>,
    added: usize,
}

/// Repair only flow collection boundaries; quoted, plain and block scalar text stays intact.
pub(super) fn normalize(content: &str) -> Cow<'_, str> {
    let mut state = FlowState {
        node_start: true,
        ..FlowState::default()
    };
    let bytes = content.as_bytes();
    let mut start = 0;
    let mut output: Option<String> = None;
    while start < content.len() {
        let mut end = start;
        while end < bytes.len() && !matches!(bytes[end], b'\r' | b'\n') {
            end += 1;
        }
        if end < bytes.len() {
            if bytes[end] == b'\r' && bytes.get(end + 1) == Some(&b'\n') {
                end += 2;
            } else {
                end += 1;
            }
        }
        let original = &content[start..end];
        let indent = original.bytes().take_while(|byte| *byte == b' ').count();
        let first = &original[indent..];
        let skip_plain = state.plain_indent.is_some_and(|required| {
            first.trim().is_empty() || first.starts_with('#') || indent > required
        });
        let skip_block = state
            .block_indent
            .is_some_and(|required| first.trim().is_empty() || indent > required);
        if skip_plain || skip_block {
            if let Some(output) = &mut output {
                output.push_str(original);
            }
            start = end;
            continue;
        }
        state.plain_indent = None;
        state.block_indent = None;
        let mut replacement = None;
        if let Some(&required) = state.indents.last() {
            if state.quote.is_none() && !first.trim().is_empty() && !first.starts_with('#') {
                if ["---", "..."].iter().any(|marker| {
                    first.strip_prefix(marker).is_some_and(|suffix| {
                        suffix.is_empty() || suffix.starts_with(char::is_whitespace)
                    })
                }) {
                    return Cow::Borrowed(content);
                }
                let boundary = matches!(state.previous, b'[' | b'{' | b',')
                    || first
                        .as_bytes()
                        .first()
                        .is_some_and(|byte| matches!(byte, b',' | b']' | b'}'));
                if boundary && indent < required && !first.starts_with('\t') {
                    let padding = required - indent;
                    state.added += padding;
                    if state.added > content.len() {
                        return Cow::Borrowed(content);
                    }
                    let mut line = String::with_capacity(padding + original.len());
                    line.extend(std::iter::repeat_n(' ', padding));
                    line.push_str(original);
                    replacement = Some(line);
                    if output.is_none() {
                        let mut accumulated = String::with_capacity(content.len() + state.added);
                        accumulated.push_str(&content[..start]);
                        output = Some(accumulated);
                    }
                }
            }
        }
        let line = replacement.as_deref().unwrap_or(original);
        if let Some(output) = &mut output {
            output.push_str(line);
        }
        if !state.scan(line) {
            return Cow::Borrowed(content);
        }
        start = end;
    }
    output.map_or(Cow::Borrowed(content), Cow::Owned)
}

impl FlowState {
    fn scan(&mut self, line: &str) -> bool {
        let bytes = line.as_bytes();
        let indent = bytes.iter().take_while(|byte| **byte == b' ').count();
        let mut sequence_indent = indent;
        while bytes.get(sequence_indent) == Some(&b'-')
            && bytes.get(sequence_indent + 1) == Some(&b' ')
        {
            sequence_indent += 2;
            while bytes.get(sequence_indent) == Some(&b' ') {
                sequence_indent += 1;
            }
        }
        let mut block_mapping = false;
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if let Some(quote) = self.quote {
                if quote == b'"' && byte == b'\\' {
                    index += 2;
                    continue;
                }
                if byte == quote {
                    if quote == b'\'' && bytes.get(index + 1) == Some(&b'\'') {
                        index += 2;
                        continue;
                    }
                    self.quote = None;
                    self.previous = byte;
                }
                index += 1;
                continue;
            }
            if byte.is_ascii_whitespace() {
                index += 1;
                continue;
            }
            if byte == b'#'
                && (index == 0
                    || bytes[index - 1].is_ascii_whitespace()
                    || matches!(bytes[index - 1], b'[' | b'{' | b','))
            {
                break;
            }
            let next = bytes.get(index + 1).copied().unwrap_or(b'\n');
            if self.node_start && matches!(byte, b'"' | b'\'') {
                self.quote = Some(byte);
                self.node_start = false;
            } else if self.node_start
                && self.indents.is_empty()
                && matches!(byte, b'|' | b'>')
                && block_scalar_header(bytes, index)
            {
                self.block_indent = Some(if sequence_indent > indent {
                    if block_mapping {
                        sequence_indent
                    } else {
                        sequence_indent - 2
                    }
                } else {
                    indent
                });
                break;
            } else if self.node_start && matches!(byte, b'!' | b'&') {
                if byte == b'!' && next == b'<' {
                    let Some(end) = bytes[index + 2..].iter().position(|byte| *byte == b'>') else {
                        return false;
                    };
                    index += end + 2;
                } else {
                    while bytes
                        .get(index + 1)
                        .is_some_and(|byte| !byte.is_ascii_whitespace() && !b"[]{},".contains(byte))
                    {
                        index += 1;
                    }
                }
            } else if self.node_start && matches!(byte, b'[' | b'{') {
                self.plain_indent = None;
                self.indents
                    .push((indent + 1).max(self.indents.last().copied().unwrap_or(0)));
                self.previous = byte;
            } else if !self.indents.is_empty() && matches!(byte, b']' | b'}') {
                self.indents.pop();
                self.node_start = false;
                self.previous = byte;
            } else if (!self.indents.is_empty() && byte == b',')
                || (byte == b':' && (next.is_ascii_whitespace() || b"[]{},".contains(&next)))
            {
                if self.indents.is_empty() {
                    block_mapping = true;
                }
                self.plain_indent = None;
                self.node_start = true;
                self.previous = byte;
            } else if self.node_start && matches!(byte, b'-' | b'?') && next.is_ascii_whitespace() {
                self.previous = byte;
            } else {
                if self.node_start && self.indents.is_empty() {
                    self.plain_indent = Some(if sequence_indent > indent {
                        if block_mapping {
                            sequence_indent
                        } else {
                            sequence_indent - 2
                        }
                    } else {
                        indent
                    });
                }
                self.node_start = false;
                self.previous = byte;
            }
            index += 1;
        }
        if self.indents.is_empty() && self.quote.is_none() {
            self.node_start = true;
            self.previous = 0;
        }
        true
    }
}

fn block_scalar_header(bytes: &[u8], position: usize) -> bool {
    let mut cursor = position + 1;
    for _ in 0..2 {
        if bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b'1'..=b'9' | b'+' | b'-'))
        {
            cursor += 1;
        }
    }
    while bytes
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    bytes
        .get(cursor)
        .is_none_or(|byte| matches!(byte, b'\r' | b'\n' | b'#'))
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn repairs_collection_boundaries_without_changing_scalar_data() {
        let input = "proxies: [\n{name: node, type: socks5, server: example.com, port: 1080}\n]\n";
        let repaired = normalize(input);
        assert_eq!(
            serde_yaml::from_str::<serde_json::Value>(&repaired).expect("repaired YAML")["proxies"]
                [0]["name"],
            "node"
        );
        for input in [
            "value: |\n  [literal,\n  content]\n",
            "value: '[literal,\ncontent]'\n",
            "value: plain\n  [literal, content]\n",
        ] {
            assert_eq!(normalize(input), input);
        }
    }
}
