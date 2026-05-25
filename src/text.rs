use crate::core::{ArtifactKind, Envelope, Hashes, LineIndex, ParserInfo, SourceInfo, SourceRange};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextDocument {
    pub schema_version: String,
    pub blocks: Vec<TextBlock>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextBlock {
    pub id: String,
    pub text: String,
    pub range: SourceRange,
}

pub type TextEnvelope = Envelope<TextDocument>;

pub fn parse_text(text: &str, source: SourceInfo) -> TextEnvelope {
    let index = LineIndex::new(text);
    let mut blocks = Vec::new();
    let mut block_start = None;
    let mut last_non_empty_end = 0;
    for (offset, line) in lines_with_offsets(text) {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.trim().is_empty() {
            if let Some(start) = block_start.take() {
                blocks.push(text_block(
                    blocks.len(),
                    text,
                    start,
                    last_non_empty_end,
                    &index,
                ));
            }
        } else {
            block_start.get_or_insert(offset);
            last_non_empty_end = offset + trimmed.len();
        }
    }
    if let Some(start) = block_start {
        blocks.push(text_block(
            blocks.len(),
            text,
            start,
            last_non_empty_end,
            &index,
        ));
    }

    Envelope::new(
        ArtifactKind::Text,
        source,
        ParserInfo::new("grist.text"),
        "grist/text/v1",
        TextDocument {
            schema_version: "grist/text/v1".to_string(),
            blocks,
        },
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
}

fn text_block(id: usize, source: &str, start: usize, end: usize, index: &LineIndex) -> TextBlock {
    TextBlock {
        id: format!("text-block-{id}"),
        text: source[start..end].trim().to_string(),
        range: SourceRange::new(start, end, index),
    }
}

fn lines_with_offsets(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    text.split_inclusive('\n').map(move |line| {
        let current = offset;
        offset += line.len();
        (current, line)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plain_text_into_blocks() {
        let report = parse_text("one\n\ntwo\nthree", SourceInfo::stdin("note.txt"));
        assert_eq!(report.payload.blocks.len(), 2);
        assert_eq!(report.payload.blocks[1].text, "two\nthree");
    }
}
